use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, MouseEventKind};
use gim::app::{App, Flow};
use gim::clipboard::default_clipboard;
use gim::file::{self, Document};
use gim::terminal::{self, TerminalGuard};
use gim::ui;

const USAGE: &str = "usage: gim [FILE]
       gim --local [FILE]
       gim --kill [FILE]

Opens FILE, or the default notes file when omitted:
$GIM_NOTES, else $XDG_DATA_HOME/gim/notes.md, else ~/.local/share/gim/notes.md.

Every window on the same file shares one editor through a daemon that
keeps running after the windows close. --local edits in this process only;
--kill saves and stops the daemon for the file.";

enum Cli {
    Open(PathBuf),
    Local(PathBuf),
    Kill(PathBuf),
    Daemon(PathBuf),
    Help,
    Usage,
}

fn parse_args(args: &[OsString]) -> Cli {
    let path = |rest: &[OsString]| match rest {
        [] => Some(file::default_notes_path()),
        [one] if !one.to_string_lossy().starts_with('-') => Some(PathBuf::from(one)),
        _ => None,
    };
    match args {
        [one] if one == "-h" || one == "--help" => Cli::Help,
        [flag, rest @ ..] if flag == "--local" => path(rest).map_or(Cli::Usage, Cli::Local),
        [flag, rest @ ..] if flag == "--kill" => path(rest).map_or(Cli::Usage, Cli::Kill),
        [flag, one] if flag == "--daemon" => Cli::Daemon(PathBuf::from(one)),
        rest => path(rest).map_or(Cli::Usage, Cli::Open),
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let result = match parse_args(&args) {
        Cli::Help => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Cli::Usage => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
        Cli::Local(path) => local(&path),
        Cli::Open(path) => open(&path),
        Cli::Kill(path) => kill(&path),
        Cli::Daemon(path) => return daemon(&path),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("gim: {e}");
            ExitCode::from(1)
        }
    }
}

fn load(path: &Path) -> io::Result<(Document, String)> {
    file::load(path).map_err(io::Error::other)
}

#[cfg(unix)]
fn target_of(path: &Path) -> io::Result<PathBuf> {
    file::canonical_target(path).map_err(|e| {
        let hint = match e.kind() {
            io::ErrorKind::NotFound => " (parent directory must exist)",
            _ => "",
        };
        io::Error::new(e.kind(), format!("{}: {e}{hint}", path.display()))
    })
}

fn local(path: &Path) -> io::Result<ExitCode> {
    if daemon_holds(path) {
        let shown = path.display();
        let text =
            format!("a daemon holds {shown}; run gim {shown} to attach or gim --kill {shown}");
        return Err(io::Error::other(text));
    }
    let (doc, text) = load(path)?;
    run(doc, text)?;
    Ok(ExitCode::SUCCESS)
}

#[cfg(unix)]
fn daemon_holds(path: &Path) -> bool {
    let Ok(target) = file::canonical_target(path) else {
        return false;
    };
    gim::daemon::paths(&target).is_ok_and(|paths| gim::daemon::probe(&paths.sock))
}

#[cfg(not(unix))]
fn daemon_holds(_path: &Path) -> bool {
    false
}

#[cfg(unix)]
fn open(path: &Path) -> io::Result<ExitCode> {
    use gim::client::Exit;
    load(path)?;
    let target = target_of(path)?;
    let paths = gim::daemon::paths(&target)?;
    let stream = gim::daemon::ensure_running(&target, &paths)?;
    match gim::client::attach(stream, &paths.sock)? {
        Exit::Detached => Ok(ExitCode::SUCCESS),
        Exit::DaemonGone => Err(io::Error::other("daemon exited")),
        Exit::Error(text) => Err(io::Error::other(text)),
    }
}

#[cfg(not(unix))]
fn open(path: &Path) -> io::Result<ExitCode> {
    local(path)
}

#[cfg(unix)]
fn kill(path: &Path) -> io::Result<ExitCode> {
    let stopped = match file::canonical_target(path) {
        Ok(target) => gim::daemon::kill(&gim::daemon::paths(&target)?)?,
        Err(_) => false,
    };
    if !stopped {
        println!("gim: no daemon for {}", path.display());
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(not(unix))]
fn kill(_path: &Path) -> io::Result<ExitCode> {
    Err(io::Error::other(NO_DAEMONS))
}

#[cfg(unix)]
fn daemon(path: &Path) -> ExitCode {
    use std::io::Write;
    match gim::daemon::run(path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = writeln!(io::stderr(), "gim daemon: {e}");
            ExitCode::from(1)
        }
    }
}

#[cfg(not(unix))]
fn daemon(_path: &Path) -> ExitCode {
    eprintln!("gim: {NO_DAEMONS}");
    ExitCode::from(1)
}

#[cfg(not(unix))]
const NO_DAEMONS: &str = "daemons are not supported on this platform; use gim --local";

fn run(doc: Document, text: String) -> io::Result<()> {
    let (guard, mut terminal) = TerminalGuard::enter()?;
    terminal::install_panic_hook(guard.state());
    let size = terminal.size()?;
    let mut app = App::new(doc, text, default_clipboard(), size.width, size.height);
    let mut needs_draw = true;
    loop {
        if needs_draw {
            terminal.draw(|frame| ui::draw(&app, frame))?;
            needs_draw = false;
        }
        let timeout = app
            .next_deadline(Instant::now())
            .unwrap_or(Duration::from_secs(3600));
        if !event::poll(timeout)? {
            needs_draw = app.advance(Instant::now());
            continue;
        }
        let ev = event::read()?;
        if let Event::Mouse(mouse) = &ev
            && mouse.kind == MouseEventKind::Moved
        {
            continue;
        }
        needs_draw = true;
        if app.handle_event(ev, Instant::now()) == Flow::Quit {
            return Ok(());
        }
    }
}
