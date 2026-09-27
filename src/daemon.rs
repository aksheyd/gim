use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, File, TryLockError};
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::app::App;
use crate::clipboard::default_clipboard;
use crate::file;
use crate::protocol::{
    self, ClientMsg, PROTOCOL_ERROR, PROTOCOL_VERSION, ServerMsg, WRITE_TIMEOUT,
};
use crate::server::{ClientId, Out, Server};

const MAX_SOCK_PATH: usize = 100;
const START_TRIES: u32 = 100;
const START_INTERVAL: Duration = Duration::from_millis(50);
const WAKE_CAP: Duration = Duration::from_secs(1);
const ACCEPT_POLL: Duration = Duration::from_millis(50);
const KILL_TIMEOUT: Duration = Duration::from_secs(10);
const STOPPED_READING: &str = "disconnected: this window stopped reading";

#[derive(Debug)]
pub struct Paths {
    pub sock: PathBuf,
    pub lock: PathBuf,
    pub log: PathBuf,
}

impl Paths {
    fn prepare(&self) -> io::Result<()> {
        let dir = self.sock.parent().unwrap_or(Path::new("."));
        let created = fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .and_then(|()| fs::set_permissions(dir, fs::Permissions::from_mode(0o700)));
        created.map_err(|e| {
            let text = format!("cannot prepare {}: {e}; use gim --local", dir.display());
            io::Error::new(e.kind(), text)
        })
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn paths(canonical: &Path) -> io::Result<Paths> {
    paths_from(
        canonical,
        std::env::var_os("XDG_STATE_HOME"),
        std::env::var_os("HOME"),
    )
}

pub fn state_dir() -> io::Result<PathBuf> {
    state_dir_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

pub fn state_dir_from(
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> io::Result<PathBuf> {
    let non_empty = |v: Option<OsString>| v.filter(|s| !s.is_empty());
    let base = match (non_empty(xdg_state_home), non_empty(home)) {
        (Some(xdg), _) => PathBuf::from(xdg),
        (None, Some(home)) => PathBuf::from(home).join(".local").join("state"),
        (None, None) => return Err(io::Error::other("HOME is not set; use gim --local")),
    };
    Ok(base.join("gim").join("run"))
}

pub fn paths_from(
    canonical: &Path,
    xdg_state_home: Option<OsString>,
    home: Option<OsString>,
) -> io::Result<Paths> {
    let dir = state_dir_from(xdg_state_home, home)?;
    let name = format!("{:016x}", fnv1a64(canonical.as_os_str().as_bytes()));
    let sock = dir.join(format!("{name}.sock"));
    if sock.as_os_str().len() > MAX_SOCK_PATH {
        return Err(io::Error::other("socket path too long; use gim --local"));
    }
    Ok(Paths {
        sock,
        lock: dir.join(format!("{name}.lock")),
        log: dir.join(format!("{name}.log")),
    })
}

pub fn probe(sock: &Path) -> bool {
    UnixStream::connect(sock).is_ok()
}

pub fn ensure_running(canonical: &Path, paths: &Paths) -> io::Result<UnixStream> {
    if let Ok(stream) = UnixStream::connect(&paths.sock) {
        return Ok(stream);
    }
    paths.prepare()?;
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&paths.lock)?;
    let mut child = None;
    match lock.try_lock() {
        Ok(()) => {
            match UnixStream::connect(&paths.sock) {
                Ok(stream) => return Ok(stream),
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                    let _ = fs::remove_file(&paths.sock);
                }
                Err(_) => {}
            }
            let spawned = spawn(canonical, paths)
                .map_err(|e| io::Error::new(e.kind(), format!("cannot start daemon: {e}")))?;
            child = Some(spawned);
        }
        Err(TryLockError::WouldBlock) => {}
        Err(TryLockError::Error(e)) => return Err(e),
    }
    for _ in 0..START_TRIES {
        thread::sleep(START_INTERVAL);
        if let Ok(stream) = UnixStream::connect(&paths.sock) {
            return Ok(stream);
        }
        if let Some(child) = &mut child
            && let Some(status) = child.try_wait()?
        {
            let log = paths.log.display();
            let text = format!("daemon exited during start ({status}); see {log}");
            return Err(io::Error::other(text));
        }
    }
    let text = format!("daemon did not start; see {}", paths.log.display());
    Err(io::Error::other(text))
}

fn spawn(canonical: &Path, paths: &Paths) -> io::Result<Child> {
    let out = File::options().create(true).append(true).open(&paths.log)?;
    let err = out.try_clone()?;
    Command::new(std::env::current_exe()?)
        .arg("--daemon")
        .arg(canonical)
        .process_group(0)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err))
        .spawn()
}

pub fn kill(paths: &Paths) -> io::Result<bool> {
    use io::ErrorKind::{ConnectionRefused, NotFound};
    let mut stream = match UnixStream::connect(&paths.sock) {
        Ok(stream) => stream,
        Err(e) if matches!(e.kind(), NotFound | ConnectionRefused) => return Ok(false),
        Err(e) => return Err(e),
    };
    stream.set_read_timeout(Some(KILL_TIMEOUT))?;
    protocol::write_msg(&mut stream, &ClientMsg::Shutdown)?;
    let answer = protocol::read_msg::<_, ServerMsg>(&mut stream).map_err(|e| match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
            let (secs, log) = (KILL_TIMEOUT.as_secs(), paths.log.display());
            let text = format!("daemon did not answer within {secs} s; see {log}");
            io::Error::new(e.kind(), text)
        }
        _ => e,
    })?;
    match answer {
        Some(ServerMsg::Bye) => Ok(true),
        Some(ServerMsg::Error(e)) => Err(io::Error::other(e)),
        Some(ServerMsg::Patch(_)) => Err(io::Error::other(PROTOCOL_ERROR)),
        None => Err(io::Error::other("daemon closed the connection")),
    }
}

pub fn run(arg: &Path) -> io::Result<()> {
    let canonical = file::canonical_target(arg)?;
    let paths = paths(&canonical)?;
    paths.prepare()?;
    let log = File::options().create(true).append(true).open(&paths.log)?;
    let (doc, text) = file::load(&canonical).map_err(io::Error::other)?;
    let Some(listener) = bind(&paths.sock)? else {
        return Ok(());
    };
    log.set_len(0)?;
    let app = App::new(doc, text, default_clipboard(), 80, 24);
    serve(app, listener, &paths.sock, log)
}

fn bind(sock: &Path) -> io::Result<Option<UnixListener>> {
    let listener = match UnixListener::bind(sock) {
        Ok(listener) => listener,
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => match UnixStream::connect(sock) {
            Ok(_) => return Ok(None),
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                let text = format!(
                    "stale socket {}; run gim on the file to clean it up",
                    sock.display()
                );
                return Err(io::Error::new(io::ErrorKind::AddrInUse, text));
            }
            Err(e) => return Err(e),
        },
        Err(e) => return Err(e),
    };
    fs::set_permissions(sock, fs::Permissions::from_mode(0o600))?;
    Ok(Some(listener))
}

fn owns(sock: &Path, ino: u64) -> bool {
    fs::symlink_metadata(sock).is_ok_and(|m| m.ino() == ino)
}

enum Incoming {
    Attached(ClientId, UnixStream),
    Msg(ClientId, ClientMsg),
    Bad(ClientId),
    Gone(ClientId),
}

struct GoneOnDrop(ClientId, Sender<Incoming>);

impl Drop for GoneOnDrop {
    fn drop(&mut self) {
        let _ = self.1.send(Incoming::Gone(self.0));
    }
}

fn read_loop(id: ClientId, mut reader: UnixStream, writer: UnixStream, tx: Sender<Incoming>) {
    let gone = GoneOnDrop(id, tx);
    if gone.1.send(Incoming::Attached(id, writer)).is_err() {
        return;
    }
    loop {
        match protocol::read_msg::<_, ClientMsg>(&mut reader) {
            Ok(Some(msg)) => {
                if gone.1.send(Incoming::Msg(id, msg)).is_err() {
                    return;
                }
            }
            Ok(None) => return,
            Err(_) => {
                let _ = gone.1.send(Incoming::Bad(id));
                return;
            }
        }
    }
}

fn accept_loop(listener: UnixListener, tx: Sender<Incoming>, stop: Arc<AtomicBool>, mut log: File) {
    let mut next_id: ClientId = 1;
    while !stop.load(Ordering::Relaxed) {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(_) => {
                thread::sleep(ACCEPT_POLL);
                continue;
            }
        };
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if stream.set_nonblocking(false).is_err()
            || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
        {
            continue;
        }
        let Ok(reader) = stream.try_clone() else {
            continue;
        };
        let id = next_id;
        next_id += 1;
        let tx = tx.clone();
        let spawned = thread::Builder::new().spawn(move || read_loop(id, reader, stream, tx));
        if let Err(e) = spawned {
            let _ = writeln!(log, "client {id}: refused, cannot start reader: {e}");
        }
    }
}

struct Conn {
    stream: UnixStream,
    live: bool,
}

type Conns = BTreeMap<ClientId, Conn>;

fn drop_conn(
    server: &mut Server,
    conns: &mut Conns,
    id: ClientId,
    reply: Option<&str>,
    now: Instant,
) -> Out {
    let Some(mut conn) = conns.remove(&id) else {
        return Vec::new();
    };
    if let Some(text) = reply {
        let _ = conn.stream.set_nonblocking(true);
        let _ = protocol::write_msg(&mut conn.stream, &ServerMsg::Error(text.to_string()));
    }
    let _ = conn.stream.shutdown(Shutdown::Both);
    if conn.live {
        server.detach(id, now)
    } else {
        Vec::new()
    }
}

fn flush(server: &mut Server, conns: &mut Conns, mut out: Out, now: Instant, log: &mut File) {
    while !out.is_empty() {
        let mut failed = Vec::new();
        for (id, msg) in out.drain(..) {
            let Some(conn) = conns.get_mut(&id) else {
                continue;
            };
            let bye = matches!(msg, ServerMsg::Bye);
            if protocol::write_msg(&mut conn.stream, &msg).is_err() {
                failed.push(id);
            } else if bye {
                let _ = writeln!(log, "client {id}: detached");
                let _ = conn.stream.shutdown(Shutdown::Both);
                conns.remove(&id);
            }
        }
        for id in failed {
            let _ = writeln!(log, "client {id}: stopped reading, dropped");
            out.extend(drop_conn(server, conns, id, Some(STOPPED_READING), now));
        }
    }
}

enum Step {
    Continue,
    Stop(&'static str),
}

fn handle(
    server: &mut Server,
    conns: &mut Conns,
    incoming: Incoming,
    now: Instant,
    log: &mut File,
) -> (Out, Step) {
    let mut out = Vec::new();
    match incoming {
        Incoming::Attached(id, stream) => {
            conns.insert(
                id,
                Conn {
                    stream,
                    live: false,
                },
            );
        }
        Incoming::Gone(id) => {
            if let Some(conn) = conns.remove(&id)
                && conn.live
            {
                let _ = writeln!(log, "client {id}: detached");
                out = server.detach(id, now);
            }
        }
        Incoming::Bad(id) if conns.contains_key(&id) => {
            let _ = writeln!(log, "client {id}: {PROTOCOL_ERROR}");
            out = drop_conn(server, conns, id, Some(PROTOCOL_ERROR), now);
        }
        Incoming::Bad(_) => {}
        Incoming::Msg(id, msg) => {
            let Some(conn) = conns.get_mut(&id) else {
                return (out, Step::Continue);
            };
            match msg {
                ClientMsg::Hello {
                    version,
                    width,
                    height,
                } if !conn.live => {
                    if version != PROTOCOL_VERSION {
                        let _ = writeln!(log, "client {id}: protocol v{version} refused");
                        let text = format!(
                            "daemon speaks protocol v{PROTOCOL_VERSION}, client v{version}; run gim --kill and retry"
                        );
                        out = drop_conn(server, conns, id, Some(&text), now);
                    } else {
                        conn.live = true;
                        let _ = writeln!(log, "client {id}: attached {width}x{height}");
                        out = server.attach(id, width, height, now);
                    }
                }
                ClientMsg::Event(event) if conn.live => out = server.event(id, event, now),
                ClientMsg::Shutdown => match server.shutdown() {
                    Ok(()) => return (out, Step::Stop("shutdown requested")),
                    Err(e) => {
                        let _ = writeln!(log, "shutdown refused: {e}");
                        let _ = protocol::write_msg(&mut conn.stream, &ServerMsg::Error(e));
                    }
                },
                _ => {
                    let _ = writeln!(log, "client {id}: {PROTOCOL_ERROR}");
                    out = drop_conn(server, conns, id, Some(PROTOCOL_ERROR), now);
                }
            }
        }
    }
    (out, Step::Continue)
}

fn serve(app: App, listener: UnixListener, sock: &Path, mut log: File) -> io::Result<()> {
    let ino = fs::symlink_metadata(sock)?.ino();
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let (accept_stop, accept_log) = (Arc::clone(&stop), log.try_clone()?);
    let accept = thread::spawn(move || accept_loop(listener, tx, accept_stop, accept_log));
    let mut server = Server::new(app);
    let mut conns = Conns::new();
    let _ = writeln!(log, "listening on {}", sock.display());
    let (reason, saved) = loop {
        let now = Instant::now();
        let wait = server.next_deadline(now).unwrap_or(WAKE_CAP).min(WAKE_CAP);
        let incoming = match rx.recv_timeout(wait) {
            Ok(incoming) => Some(incoming),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break ("accept thread ended", false),
        };
        if !owns(sock, ino) {
            break ("socket removed or replaced", false);
        }
        let now = Instant::now();
        let (out, step) = match incoming {
            Some(incoming) => handle(&mut server, &mut conns, incoming, now, &mut log),
            None => (server.tick(now), Step::Continue),
        };
        flush(&mut server, &mut conns, out, now, &mut log);
        if let Step::Stop(reason) = step {
            break (reason, true);
        }
    };
    if !saved && let Err(e) = server.shutdown() {
        let _ = writeln!(log, "save failed on exit: {e}");
    }
    if owns(sock, ino) {
        let _ = fs::remove_file(sock);
    }
    for conn in conns.values_mut() {
        let _ = protocol::write_msg(&mut conn.stream, &ServerMsg::Bye);
    }
    for conn in conns.values() {
        let _ = conn.stream.shutdown(Shutdown::Both);
    }
    stop.store(true, Ordering::Relaxed);
    let _ = accept.join();
    let _ = writeln!(log, "exit: {reason}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::thread::JoinHandle;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::clipboard::MemClipboard;
    use crate::file::temp_dir;
    use crate::protocol::Patch;

    struct Daemon {
        paths: Paths,
        notes: PathBuf,
        handle: JoinHandle<io::Result<()>>,
    }

    impl Daemon {
        fn start(text: &str) -> Self {
            let dir = temp_dir();
            let paths = Paths {
                sock: dir.join("d.sock"),
                lock: dir.join("d.lock"),
                log: dir.join("d.log"),
            };
            let notes = dir.join("notes.md");
            fs::write(&notes, text).unwrap();
            let listener = bind(&paths.sock).unwrap().unwrap();
            let log = File::create(&paths.log).unwrap();
            let handle = thread::spawn({
                let (sock, notes) = (paths.sock.clone(), notes.clone());
                move || {
                    let (doc, text) = file::load(&notes).unwrap();
                    let clipboard = Box::new(MemClipboard::default());
                    let app = App::new(doc, text, clipboard, 80, 24);
                    serve(app, listener, &sock, log)
                }
            });
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
            self.handle.join().unwrap().unwrap();
            let log = fs::read_to_string(&self.paths.log).unwrap();
            let _ = fs::remove_dir_all(self.paths.sock.parent().unwrap());
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

        assert!(kill(&d.paths).unwrap());
        assert_eq!(next(&mut b), ServerMsg::Bye);
        assert!(!d.paths.sock.exists());
        assert_eq!(fs::read_to_string(&d.notes).unwrap(), "xyhi");
        assert!(!kill(&d.paths).unwrap());
        let log = d.finish();
        assert!(log.contains("client 1: attached 40x10"));
        assert!(log.contains("client 1: detached"));
        assert!(log.contains("exit: shutdown requested"));
        assert!(!log.contains("client 3"));
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

        assert!(kill(&d.paths).unwrap());
        assert_eq!(next(&mut b), ServerMsg::Bye);
        let log = d.finish();
        assert!(log.contains("client 1: protocol error"));
    }

    #[test]
    fn removing_the_socket_stops_the_daemon_after_saving() {
        let d = Daemon::start("");
        let mut a = d.hello(40, 10);
        patch(&mut a);
        key(&mut a, 'q');
        patch(&mut a);
        let started = Instant::now();
        fs::remove_file(&d.paths.sock).unwrap();
        assert_eq!(next(&mut a), ServerMsg::Bye);
        assert!(started.elapsed() < WAKE_CAP * 3);
        assert_eq!(fs::read_to_string(&d.notes).unwrap(), "q");
        let log = d.finish();
        assert!(log.contains("exit: socket removed or replaced"));
    }

    #[test]
    fn a_replaced_socket_is_left_alone() {
        let d = Daemon::start("");
        let mut a = d.hello(40, 10);
        patch(&mut a);
        fs::remove_file(&d.paths.sock).unwrap();
        fs::write(&d.paths.sock, "successor").unwrap();
        assert_eq!(next(&mut a), ServerMsg::Bye);
        assert_eq!(protocol::read_msg::<_, ServerMsg>(&mut a).unwrap(), None);
        assert_eq!(fs::read_to_string(&d.paths.sock).unwrap(), "successor");
        d.finish();
    }

    #[test]
    fn paths_are_stable_private_and_need_a_home() {
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        let dir = PathBuf::from("/tmp").join(format!("gim-paths-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let some = |p: &Path| Some(p.as_os_str().to_os_string());
        let paths = paths_from(Path::new("/n/notes.md"), some(&dir), None).unwrap();
        let run = dir.join("gim").join("run");
        assert_eq!(state_dir_from(some(&dir), None).unwrap(), run);
        assert_eq!(paths.sock, run.join("5331277a5d5cc8fd.sock"));
        assert_eq!(paths.lock, run.join("5331277a5d5cc8fd.lock"));
        assert_eq!(paths.log, run.join("5331277a5d5cc8fd.log"));
        paths.prepare().unwrap();
        let mode = fs::metadata(&run).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let paths = paths_from(Path::new("/n/notes.md"), None, some(&dir)).unwrap();
        assert!(paths.sock.starts_with(dir.join(".local").join("state")));
        let err = paths_from(Path::new("/n/notes.md"), None, None).unwrap_err();
        assert!(err.to_string().contains("--local"));
        let _ = fs::remove_dir_all(&dir);
    }
}
