//! The shared editor behind the daemon: one `App`, one off-screen terminal
//! at the smallest attached size, and the messages each client should get.
//! Pure: ids, events and times in; messages out. No sockets, threads or clocks.

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

/// Largest dimension a client may claim; a buggy one must not force a giant screen.
const MAX_DIM: u16 = 1000;

pub struct Server {
    app: App,
    screen: Terminal<TestBackend>,
    clients: BTreeMap<ClientId, (u16, u16)>,
    size: (u16, u16),
    /// The picture every client holds; `None` while nobody is attached.
    last: Option<Buffer>,
    last_cursor: Option<(u16, u16)>,
}

impl Server {
    pub fn new(app: App) -> Self {
        // An in-memory backend cannot fail, so these `Ok` patterns are irrefutable.
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

    /// Adds a client; a newcomer that does not change the shared size only
    /// receives the current picture, everyone else hears nothing.
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
        // The departed window can never send its mouse-up.
        self.app.editor.end_drag();
        self.relayout(now)
    }

    /// Feeds one client's event to the editor; `Resize` only updates that
    /// client's size, and a quit detaches just the sender.
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

    /// Saves; the caller says goodbye only when this succeeds.
    pub fn shutdown(&mut self) -> Result<(), String> {
        self.app.save().map(|_| ())
    }

    fn shared_size(&self) -> Option<(u16, u16)> {
        let width = self.clients.values().map(|s| s.0).min()?;
        let height = self.clients.values().map(|s| s.1).min()?;
        Some((width, height))
    }

    /// Re-renders after the attached set or a client size changed.
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

    /// Renders and sends what changed to everyone; with no clients the
    /// picture is dropped so the next attach starts from a full patch.
    fn broadcast(&mut self) -> Out {
        if self.clients.is_empty() {
            self.last = None;
            self.last_cursor = None;
            return Vec::new();
        }
        let app = &self.app;
        // The completed frame is the real picture; the backend keeps stale cells after wide glyphs.
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
    use crate::app::{AUTOSAVE_DELAY, Mode, QUIT_PROMPT};
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

    /// Points the document at a path whose parent is a regular file, so saves fail.
    fn block_saves(server: &mut Server) -> std::path::PathBuf {
        let blocker = server.app.doc.path.parent().unwrap().join("blocker");
        fs::write(&blocker, "").unwrap();
        server.app.doc.path = blocker.join("child.md");
        blocker
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

    /// Applies each message to its client; `Bye` drops the client.
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

    /// Every attached client's mirror equals a fresh, independent render of
    /// the app at the shared size, so a wrong buffer inside `broadcast` shows.
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

        // A bigger newcomer gets the standing picture; A hears nothing.
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

    #[test]
    fn failed_save_prompt_is_shared_and_answered_by_anyone() {
        let mut s = make_server("");
        let blocker = block_saves(&mut s);
        let t = Instant::now();
        let mut m = Mirrors::new();
        deliver(&mut m, &s.attach(A, 60, 10, t));
        deliver(&mut m, &s.attach(B, 60, 10, t));
        deliver(&mut m, &s.event(A, plain('x'), t));
        let out = s.event(A, ctrl('q'), t);
        assert_eq!(patches(&out).len(), 2);
        deliver(&mut m, &out);
        check(&s, &m);
        assert_eq!(s.app.mode(), Mode::QuitPrompt);
        let status = row(&m, B, 9);
        assert!(status.contains(&QUIT_PROMPT[..8]), "{status:?}");

        let out = s.event(B, plain('n'), t);
        assert_eq!(out[0], (B, ServerMsg::Bye));
        deliver(&mut m, &out);
        check(&s, &m);
        assert_eq!(s.app.mode(), Mode::Normal);
        assert_eq!(s.clients(), 1);
        let status = row(&m, A, 9);
        assert!(!status.contains(&QUIT_PROMPT[..8]), "{status:?}");
        assert_eq!(s.app.editor.text(), "x");
        let _ = fs::remove_dir_all(blocker.parent().unwrap());
    }

    #[test]
    fn autosave_runs_without_clients_and_shutdown_needs_a_save() {
        let mut s = make_server("");
        let t = Instant::now();
        s.attach(A, 40, 10, t);
        s.event(A, plain('x'), t);
        assert!(s.detach(A, t).is_empty());
        assert_eq!(s.clients(), 0);
        assert!(s.last.is_none());
        assert_eq!(s.next_deadline(t), Some(AUTOSAVE_DELAY));
        assert!(s.tick(t + AUTOSAVE_DELAY / 2).is_empty());
        assert!(s.app.is_dirty());
        assert!(s.tick(t + AUTOSAVE_DELAY).is_empty());
        assert!(!s.app.is_dirty());
        assert_eq!(fs::read_to_string(&s.app.doc.path).unwrap(), "x");
        let _ = fs::remove_dir_all(s.app.doc.path.parent().unwrap());

        let mut s = make_server("");
        let blocker = block_saves(&mut s);
        s.attach(A, 40, 10, t);
        s.attach(B, 40, 10, t);
        s.event(A, plain('y'), t);
        assert!(s.shutdown().is_err());
        assert_eq!(s.clients(), 2);
        fs::remove_file(&blocker).unwrap();
        assert_eq!(s.shutdown(), Ok(()));
        assert_eq!(fs::read_to_string(&s.app.doc.path).unwrap(), "y");
        let _ = fs::remove_dir_all(blocker.parent().unwrap());
    }

    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as usize
        }
    }

    #[test]
    fn patches_reproduce_an_independent_render() {
        let mut s = make_server("ab");
        let t = Instant::now();
        let mut m = Mirrors::new();
        deliver(&mut m, &s.attach(A, 8, 3, t));
        let go = |s: &mut Server, m: &mut Mirrors, ev: Event| {
            deliver(m, &s.event(A, ev, t));
            check(s, m);
        };
        // Select-all, a wide glyph, then ASCII over it: the stale-cell trap.
        go(&mut s, &mut m, key(KeyCode::Char('a'), KeyModifiers::ALT));
        assert!(look(&m[&A].buf.as_ref().unwrap()[(0, 0)]).1);
        go(&mut s, &mut m, plain('コ'));
        go(&mut s, &mut m, plain('x'));
        // `コ` owns two cells, so its trailing blank shows as a space before the `x`.
        assert_eq!(row(&m, A, 0), "コ x     ");
        go(&mut s, &mut m, key(KeyCode::Char('a'), KeyModifiers::ALT));
        go(&mut s, &mut m, plain('x'));
        assert_eq!(row(&m, A, 0), "x       ");

        let alphabet = ["\n", "\t", " ", "a", "e\u{301}", "한", "👩\u{200d}💻"];
        let mut rng = Lcg(7);
        for _ in 0..300 {
            let ev = match rng.below(10) {
                0..=4 => {
                    let g = alphabet[rng.below(alphabet.len())];
                    let mut chars = g.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) => plain(c),
                        _ => Event::Paste(g.to_string()),
                    }
                }
                5 => key(KeyCode::Backspace, KeyModifiers::NONE),
                6 => key(KeyCode::Left, KeyModifiers::SHIFT),
                7 => key(KeyCode::Right, KeyModifiers::SHIFT),
                8 => ctrl('z'),
                _ => Event::Resize(1 + rng.below(14) as u16, 1 + rng.below(5) as u16),
            };
            go(&mut s, &mut m, ev);
        }
        let _ = fs::remove_dir_all(s.app.doc.path.parent().unwrap());
    }
}
