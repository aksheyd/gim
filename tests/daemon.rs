mod common;
use common::temp_dir;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use gim::daemon::{self, Paths};
use gim::protocol::{self, ClientMsg, PROTOCOL_ERROR, PROTOCOL_VERSION, Patch, ServerMsg};
use gim::{Event, KeyCode, KeyEvent, KeyModifiers};

struct Daemon {
    paths: Paths,
    notes: PathBuf,
    handle: JoinHandle<io::Result<()>>,
}

impl Daemon {
    fn start(text: &str) -> Self {
        let dir = temp_dir();
        let notes = dir.join("notes.md");
        std::fs::write(&notes, text).unwrap();
        let canonical = std::fs::canonicalize(&notes).unwrap();
        let paths = daemon::paths(&canonical).unwrap();
        let handle = thread::spawn({
            let notes = notes.clone();
            move || daemon::run(&notes)
        });
        for _ in 0..100 {
            if daemon::probe(&paths.sock) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(daemon::probe(&paths.sock), "daemon did not start");
        Daemon {
            paths,
            notes,
            handle,
        }
    }

    fn hello(&self, width: u16, height: u16) -> UnixStream {
        let mut stream = UnixStream::connect(&self.paths.sock).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let hello = ClientMsg::Hello {
            version: PROTOCOL_VERSION,
            width,
            height,
        };
        protocol::write_msg(&mut stream, &hello).unwrap();
        stream
    }

    fn finish(self) -> String {
        let _ = self.handle.join();
        let log = std::fs::read_to_string(&self.paths.log).unwrap_or_default();
        let _ = std::fs::remove_dir_all(self.notes.parent().unwrap());
        log
    }
}

fn next(stream: &mut UnixStream) -> ServerMsg {
    protocol::read_msg(stream).unwrap().unwrap()
}

fn patch(stream: &mut UnixStream) -> Patch {
    match next(stream) {
        ServerMsg::Patch(p) => p,
        other => panic!("expected patch, got {other:?}"),
    }
}

fn quiet(stream: &mut UnixStream) {
    stream
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    let mut byte = [0u8; 1];
    let err = stream.read(&mut byte).unwrap_err();
    assert!(matches!(
        err.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ));
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
}

fn key(stream: &mut UnixStream, c: char) {
    let event = Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    protocol::write_msg(stream, &ClientMsg::Event(event)).unwrap();
}

#[test]
fn clients_share_one_screen_and_shutdown_needs_no_hello() {
    let d = Daemon::start("hi");
    let mut a = d.hello(40, 10);
    let first = patch(&mut a);
    assert!(first.full);
    assert_eq!((first.width, first.height), (40, 10));
    let mut b = d.hello(60, 20);
    let second = patch(&mut b);
    assert!(second.full);
    assert_eq!((second.width, second.height), (40, 10));
    quiet(&mut a);

    key(&mut a, 'x');
    let (pa, pb) = (patch(&mut a), patch(&mut b));
    assert_eq!(pa, pb);
    assert!(!pa.full);

    drop(UnixStream::connect(&d.paths.sock).unwrap());
    key(&mut b, 'y');
    assert!(!patch(&mut a).full);
    assert!(!patch(&mut b).full);

    let quit = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL));
    protocol::write_msg(&mut a, &ClientMsg::Event(quit)).unwrap();
    let late = Event::Key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE));
    let _ = protocol::write_msg(&mut a, &ClientMsg::Event(late));
    assert_eq!(next(&mut a), ServerMsg::Bye);
    assert_eq!(protocol::read_msg::<_, ServerMsg>(&mut a).unwrap(), None);
    let grown = patch(&mut b);
    assert!(grown.full);
    assert_eq!((grown.width, grown.height), (60, 20));

    assert!(daemon::kill(&d.paths).unwrap());
    assert_eq!(next(&mut b), ServerMsg::Bye);
    assert!(!d.paths.sock.exists());
    assert_eq!(std::fs::read_to_string(&d.notes).unwrap(), "xyhi");
    assert!(!daemon::kill(&d.paths).unwrap());
    let _ = d.finish();
}

#[test]
fn a_malformed_message_costs_only_that_client() {
    let d = Daemon::start("");
    let mut a = d.hello(40, 10);
    let mut b = d.hello(40, 10);
    patch(&mut a);
    patch(&mut b);
    let mut garbage = Vec::from(4u32.to_be_bytes());
    garbage.extend_from_slice(b"}}}}");
    a.write_all(&garbage).unwrap();
    assert_eq!(next(&mut a), ServerMsg::Error(PROTOCOL_ERROR.to_string()));
    assert_eq!(protocol::read_msg::<_, ServerMsg>(&mut a).unwrap(), None);
    key(&mut b, 'z');
    assert!(!patch(&mut b).full);

    let mut c = UnixStream::connect(&d.paths.sock).unwrap();
    let event = ClientMsg::Event(Event::FocusGained);
    protocol::write_msg(&mut c, &event).unwrap();
    assert_eq!(next(&mut c), ServerMsg::Error(PROTOCOL_ERROR.to_string()));
    let mut v = UnixStream::connect(&d.paths.sock).unwrap();
    let hello = ClientMsg::Hello {
        version: PROTOCOL_VERSION + 1,
        width: 1,
        height: 1,
    };
    protocol::write_msg(&mut v, &hello).unwrap();
    assert!(matches!(next(&mut v), ServerMsg::Error(e) if e.contains("--kill")));
    quiet(&mut b);

    assert!(daemon::kill(&d.paths).unwrap());
    assert_eq!(next(&mut b), ServerMsg::Bye);
    let _ = d.finish();
}

#[test]
fn removing_the_socket_stops_the_daemon_after_saving() {
    let d = Daemon::start("");
    let mut a = d.hello(40, 10);
    patch(&mut a);
    key(&mut a, 'q');
    patch(&mut a);
    let started = Instant::now();
    std::fs::remove_file(&d.paths.sock).unwrap();
    assert_eq!(next(&mut a), ServerMsg::Bye);
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(std::fs::read_to_string(&d.notes).unwrap(), "q");
    let _ = d.finish();
}

#[test]
fn paths_are_stable_and_need_a_home() {
    let dir = temp_dir();
    let some = |p: &Path| Some(p.as_os_str().to_os_string());
    let paths = daemon::paths_from(Path::new("/n/notes.md"), some(&dir), None).unwrap();
    let run = dir.join("gim").join("run");
    assert_eq!(daemon::state_dir_from(some(&dir), None).unwrap(), run);
    assert_eq!(paths.sock, run.join("5331277a5d5cc8fd.sock"));
    let err = daemon::paths_from(Path::new("/n/notes.md"), None, None).unwrap_err();
    assert!(err.to_string().contains("--local"));
    let _ = std::fs::remove_dir_all(&dir);
}
