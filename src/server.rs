use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crossterm::event::Event;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::app::{App, Flow};
use crate::protocol::{self, ServerMsg};
use crate::ui;

pub type ClientId = u64;
pub type Out = Vec<(ClientId, ServerMsg)>;

const MAX_DIM: u16 = 1000;

pub struct Server {
    app: App,
    screen: Terminal<TestBackend>,
    clients: BTreeMap<ClientId, (u16, u16)>,
    size: (u16, u16),
    last: Option<Buffer>,
    last_cursor: Option<(u16, u16)>,
}

impl Server {
    pub fn new(app: App) -> Self {
        let Ok(screen) = Terminal::new(TestBackend::new(1, 1));
        Server {
            app,
            screen,
            clients: BTreeMap::new(),
            size: (0, 0),
            last: None,
            last_cursor: None,
        }
    }

    pub fn clients(&self) -> usize {
        self.clients.len()
    }

    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        self.app.next_deadline(now)
    }

    pub fn attach(&mut self, id: ClientId, width: u16, height: u16, now: Instant) -> Out {
        self.clients
            .insert(id, (width.clamp(1, MAX_DIM), height.clamp(1, MAX_DIM)));
        if self.shared_size() == Some(self.size)
            && let Some(last) = &self.last
        {
            let patch = protocol::diff(None, None, last, self.last_cursor);
            return patch
                .map(|p| (id, ServerMsg::Patch(p)))
                .into_iter()
                .collect();
        }
        self.relayout(now)
    }

    pub fn detach(&mut self, id: ClientId, now: Instant) -> Out {
        if self.clients.remove(&id).is_none() {
            return Vec::new();
        }
        self.app.editor.end_drag();
        self.relayout(now)
    }

    pub fn event(&mut self, id: ClientId, event: Event, now: Instant) -> Out {
        if !self.clients.contains_key(&id) {
            return Vec::new();
        }
        if let Event::Resize(width, height) = event {
            self.clients
                .insert(id, (width.clamp(1, MAX_DIM), height.clamp(1, MAX_DIM)));
            return self.relayout(now);
        }
        if self.app.handle_event(event, now) == Flow::Quit {
            let mut out = vec![(id, ServerMsg::Bye)];
            out.extend(self.detach(id, now));
            return out;
        }
        self.broadcast()
    }

    pub fn tick(&mut self, now: Instant) -> Out {
        if self.app.advance(now) {
            self.broadcast()
        } else {
            Vec::new()
        }
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        self.app.save().map(|_| ())
    }

    fn shared_size(&self) -> Option<(u16, u16)> {
        let width = self.clients.values().map(|s| s.0).min()?;
        let height = self.clients.values().map(|s| s.1).min()?;
        Some((width, height))
    }

    fn relayout(&mut self, now: Instant) -> Out {
        let Some(size) = self.shared_size() else {
            return self.broadcast();
        };
        if size != self.size || self.last.is_none() {
            self.size = size;
            self.app.handle_event(Event::Resize(size.0, size.1), now);
            let Ok(screen) = Terminal::new(TestBackend::new(size.0, size.1));
            self.screen = screen;
            self.last = None;
        }
        self.broadcast()
    }

    fn broadcast(&mut self) -> Out {
        if self.clients.is_empty() {
            self.last = None;
            self.last_cursor = None;
            return Vec::new();
        }
        let app = &self.app;
        let Ok(done) = self.screen.draw(|f| ui::draw(app, f));
        let next = done.buffer.clone();
        let backend = self.screen.backend();
        let visible = backend.cursor_visible();
        let cursor = visible
            .then(|| backend.cursor_position())
            .map(|p| (p.x, p.y));
        let patch = protocol::diff(self.last.as_ref(), self.last_cursor, &next, cursor);
        self.last = Some(next);
        self.last_cursor = cursor;
        match patch {
            Some(patch) => self
                .clients
                .keys()
                .map(|&id| (id, ServerMsg::Patch(patch.clone())))
                .collect(),
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;

    use super::*;
    use crate::clipboard::MemClipboard;
    use crate::file::{Document, temp_dir};
    use crate::protocol::{Patch, apply};

    const A: ClientId = 1;
    const B: ClientId = 2;
    const C: ClientId = 3;

    fn make_server(text: &str) -> Server {
        let doc = Document::new(&temp_dir().join("notes.md"));
        let clipboard = Box::new(MemClipboard::default());
        Server::new(App::new(doc, text.to_string(), clipboard, 80, 24))
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, mods))
    }

    fn plain(c: char) -> Event {
        key(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> Event {
        key(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[derive(Default)]
    struct Mirror {
        buf: Option<Buffer>,
        cursor: Option<(u16, u16)>,
    }

    type Mirrors = BTreeMap<ClientId, Mirror>;

    fn deliver(mirrors: &mut Mirrors, out: &Out) {
        for (id, msg) in out {
            match msg {
                ServerMsg::Patch(patch) => {
                    let mirror = mirrors.entry(*id).or_default();
                    let buf = mirror
                        .buf
                        .get_or_insert_with(|| Buffer::empty(Rect::new(0, 0, 1, 1)));
                    apply(patch, buf);
                    mirror.cursor = patch.cursor;
                }
                ServerMsg::Bye => {
                    mirrors.remove(id);
                }
                ServerMsg::Error(e) => panic!("unexpected error {e}"),
            }
        }
    }

    fn check(server: &Server, mirrors: &Mirrors) {
        let Ok(mut fresh) = Terminal::new(TestBackend::new(server.size.0, server.size.1));
        let Ok(done) = fresh.draw(|f| ui::draw(&server.app, f));
        let want = done.buffer.clone();
        let backend = fresh.backend();
        let visible = backend.cursor_visible();
        let want_cursor = visible
            .then(|| backend.cursor_position())
            .map(|p| (p.x, p.y));
        assert_eq!(mirrors.len(), server.clients());
        for id in server.clients.keys() {
            let mirror = &mirrors[id];
            let buf = mirror.buf.as_ref().unwrap();
            assert_eq!(buf.area, want.area, "client {id}");
            for y in 0..want.area.height {
                for x in 0..want.area.width {
                    let (a, b) = (&buf[(x, y)], &want[(x, y)]);
                    assert_eq!(look(a), look(b), "client {id} cell {x},{y}");
                }
            }
            assert_eq!(mirror.cursor, want_cursor, "client {id}");
        }
    }

    fn look(cell: &ratatui::buffer::Cell) -> (&str, bool) {
        (cell.symbol(), cell.modifier.contains(Modifier::REVERSED))
    }

    fn patches(out: &Out) -> Vec<(ClientId, &Patch)> {
        out.iter()
            .filter_map(|(id, msg)| match msg {
                ServerMsg::Patch(p) => Some((*id, p)),
                _ => None,
            })
            .collect()
    }

    fn row(m: &Mirrors, id: ClientId, y: u16) -> String {
        let buf = m[&id].buf.as_ref().unwrap();
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn clients_share_the_smallest_size_and_identical_patches() {
        let mut s = make_server("hello");
        let t = Instant::now();
        let mut m = Mirrors::new();
        let out = s.attach(A, 40, 10, t);
        let p = patches(&out);
        assert_eq!(p.len(), 1);
        assert!(p[0].1.full);
        assert_eq!((p[0].1.width, p[0].1.height), (40, 10));
        deliver(&mut m, &out);
        check(&s, &m);

        let out = s.attach(B, 60, 20, t);
        let p = patches(&out);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, B);
        assert!(p[0].1.full);
        assert_eq!((p[0].1.width, p[0].1.height), (40, 10));
        deliver(&mut m, &out);
        check(&s, &m);

        let out = s.event(A, plain('x'), t);
        let p = patches(&out);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].1, p[1].1);
        assert!(!p[0].1.full);
        deliver(&mut m, &out);
        check(&s, &m);
        assert_eq!(s.app.editor.text(), "xhello");
        assert_eq!(row(&m, B, 0).trim_end(), "xhello");

        let out = s.event(B, Event::Resize(30, 5), t);
        let p = patches(&out);
        assert_eq!(p.len(), 2);
        assert!(
            p.iter()
                .all(|(_, p)| p.full && (p.width, p.height) == (30, 5))
        );
        assert_eq!(s.app.text_rect(), Rect::new(0, 0, 30, 4));
        deliver(&mut m, &out);
        check(&s, &m);

        let out = s.detach(B, t);
        let p = patches(&out);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, A);
        assert!(p[0].1.full);
        assert_eq!((p[0].1.width, p[0].1.height), (40, 10));
        m.remove(&B);
        deliver(&mut m, &out);
        check(&s, &m);

        let out = s.attach(C, 50, 12, t);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, C);
        deliver(&mut m, &out);
        check(&s, &m);
        assert!(s.event(C, Event::Resize(0, 0), t).iter().all(|(_, msg)| {
            matches!(msg, ServerMsg::Patch(p) if p.full && (p.width, p.height) == (1, 1))
        }));
        assert_eq!(s.clients(), 2);
        let _ = fs::remove_dir_all(s.app.doc.path.parent().unwrap());
    }

    #[test]
    fn quit_detaches_only_the_sender_after_saving() {
        let mut s = make_server("");
        let t = Instant::now();
        let mut m = Mirrors::new();
        deliver(&mut m, &s.attach(A, 40, 10, t));
        deliver(&mut m, &s.attach(B, 40, 10, t));
        deliver(&mut m, &s.event(A, plain('x'), t));
        assert!(row(&m, B, 9).contains("[+]"));
        let out = s.event(A, ctrl('q'), t);
        assert_eq!(out[0], (A, ServerMsg::Bye));
        assert!(out[1..].iter().all(|(id, _)| *id == B));
        deliver(&mut m, &out);
        assert_eq!(s.clients(), 1);
        check(&s, &m);
        assert_eq!(fs::read_to_string(&s.app.doc.path).unwrap(), "x");
        assert!(!row(&m, B, 9).contains("[+]"));
        let _ = fs::remove_dir_all(s.app.doc.path.parent().unwrap());
    }
}
