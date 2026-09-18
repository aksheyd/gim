use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, MouseEventKind};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};

use crate::protocol::{self, ClientMsg, PROTOCOL_VERSION, ServerMsg, WRITE_TIMEOUT};
use crate::terminal::{self, TerminalGuard};

const FIRST_PATCH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
pub enum Exit {
    Detached,
    DaemonGone,
    Error(String),
}

enum Input {
    Local(Event),
    Remote(ServerMsg),
    Eof,
    Lost,
}

fn peer_closed(kind: io::ErrorKind) -> bool {
    matches!(
        kind,
        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset | io::ErrorKind::NotConnected
    )
}

fn stalled(kind: io::ErrorKind) -> bool {
    matches!(kind, io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

fn not_responding(sock: &Path) -> String {
    format!("daemon not responding; remove {} and retry", sock.display())
}

fn forward_terminal(tx: Sender<Input>) {
    while let Ok(ev) = event::read() {
        if let Event::Mouse(mouse) = &ev
            && mouse.kind == MouseEventKind::Moved
        {
            continue;
        }
        if tx.send(Input::Local(ev)).is_err() {
            return;
        }
    }
}

fn forward_socket(mut reader: UnixStream, tx: Sender<Input>) {
    loop {
        let input = match protocol::read_msg::<_, ServerMsg>(&mut reader) {
            Ok(Some(msg)) => Input::Remote(msg),
            Ok(None) => Input::Eof,
            Err(_) => Input::Lost,
        };
        let last = matches!(input, Input::Eof | Input::Lost);
        if tx.send(input).is_err() || last {
            return;
        }
    }
}

pub fn attach(mut stream: UnixStream, sock: &Path) -> io::Result<Exit> {
    let (guard, mut terminal) = TerminalGuard::enter()?;
    terminal::install_panic_hook(guard.state());
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let (tx, rx) = mpsc::channel();
    let terminal_tx = tx.clone();
    thread::spawn(move || forward_terminal(terminal_tx));
    let reader = stream.try_clone()?;
    thread::spawn(move || forward_socket(reader, tx));
    let size = terminal.size()?;
    let hello = ClientMsg::Hello {
        version: PROTOCOL_VERSION,
        width: size.width,
        height: size.height,
    };
    match protocol::write_msg(&mut stream, &hello) {
        Ok(()) => {}
        Err(e) if peer_closed(e.kind()) => {}
        Err(e) => return Err(e),
    }
    let first_patch_by = Instant::now() + FIRST_PATCH_TIMEOUT;

    let mut mirror = Buffer::empty(Rect::new(0, 0, 1, 1));
    let mut cursor = None;
    let mut dirty = false;
    let mut painted = false;
    loop {
        if dirty {
            terminal.draw(|frame| paint(frame, &mirror, cursor))?;
            dirty = false;
        }
        let wait = match first_patch_by.checked_duration_since(Instant::now()) {
            _ if painted => FIRST_PATCH_TIMEOUT,
            Some(left) => left,
            None => return Ok(Exit::Error(not_responding(sock))),
        };
        let first = match rx.recv_timeout(wait) {
            Ok(input) => input,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return Ok(Exit::DaemonGone),
        };
        let inputs: Vec<Input> = std::iter::once(first).chain(rx.try_iter()).collect();
        for input in inputs {
            match input {
                Input::Remote(ServerMsg::Patch(patch)) => {
                    protocol::apply(&patch, &mut mirror);
                    cursor = patch.cursor;
                    dirty = true;
                    painted = true;
                }
                Input::Remote(ServerMsg::Bye) => return Ok(Exit::Detached),
                Input::Remote(ServerMsg::Error(text)) => return Ok(Exit::Error(text)),
                Input::Eof => return Ok(Exit::DaemonGone),
                Input::Lost => return Ok(Exit::Error("connection to daemon lost".to_string())),
                Input::Local(ev) => {
                    if matches!(ev, Event::Resize(..)) {
                        dirty = true;
                    }
                    let paste = matches!(&ev, Event::Paste(_));
                    match protocol::write_msg(&mut stream, &ClientMsg::Event(ev)) {
                        Ok(()) => {}
                        Err(e) if paste && e.kind() == io::ErrorKind::InvalidData => {}
                        Err(e) if peer_closed(e.kind()) => {}
                        Err(e) if stalled(e.kind()) => {
                            return Ok(Exit::Error(not_responding(sock)));
                        }
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }
}

fn paint(frame: &mut Frame, mirror: &Buffer, cursor: Option<(u16, u16)>) {
    let area = frame.area();
    let width = mirror.area.width.min(area.width);
    let height = mirror.area.height.min(area.height);
    let buf = frame.buffer_mut();
    for y in 0..height {
        for x in 0..width {
            if let (Some(src), Some(dst)) =
                (mirror.cell((x, y)), buf.cell_mut((area.x + x, area.y + y)))
            {
                *dst = src.clone();
            }
        }
    }
    if let Some((x, y)) = cursor
        && x < area.width
        && y < area.height
    {
        frame.set_cursor_position(Position::new(area.x + x, area.y + y));
    }
}

#[cfg(test)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::protocol::{CellUpdate, Patch};

    #[test]
    fn painting_a_larger_mirror_into_a_smaller_terminal_clips() {
        let mut mirror = Buffer::empty(Rect::new(0, 0, 1, 1));
        let patch = Patch {
            width: 40,
            height: 10,
            full: true,
            cells: vec![
                CellUpdate(0, 0, "a".to_string(), false),
                CellUpdate(35, 8, "z".to_string(), true),
            ],
            cursor: Some((35, 8)),
        };
        protocol::apply(&patch, &mut mirror);
        let mut terminal = Terminal::new(TestBackend::new(30, 5)).unwrap();
        terminal
            .draw(|frame| paint(frame, &mirror, patch.cursor))
            .unwrap();
        assert_eq!(terminal.backend().buffer()[(0, 0)].symbol(), "a");
        assert!(!terminal.backend().cursor_visible());
        terminal
            .draw(|frame| paint(frame, &mirror, Some((2, 3))))
            .unwrap();
        assert!(terminal.backend().cursor_visible());
        assert_eq!(terminal.backend().cursor_position(), Position::new(2, 3));
        let mut big = Terminal::new(TestBackend::new(50, 12)).unwrap();
        big.draw(|frame| paint(frame, &mirror, patch.cursor))
            .unwrap();
        assert_eq!(big.backend().buffer()[(35, 8)].symbol(), "z");
        assert_eq!(big.backend().buffer()[(45, 11)].symbol(), " ");
        assert_eq!(big.backend().cursor_position(), Position::new(35, 8));
    }
}
