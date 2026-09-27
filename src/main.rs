use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, MouseEventKind};
use gim::app::{App, Flow};
use gim::clipboard::default_clipboard;
use gim::file::{self, Document};
use gim::terminal::{self, TerminalGuard};
use gim::ui;

#[derive(Parser)]
#[command(
    name = "gim",
    version,
    about = "A small no-modes terminal editor with emacs/macOS keybinds",
    disable_help_subcommand = true,
    args_conflicts_with_subcommands = true,
    after_help = "Every window on the same file shares one editor through a daemon that keeps running after the windows close.\n\n--local edits in this process only. --kill saves and stops the daemon for the file."
)]
struct Args {
    #[arg(
        value_name = "FILE",
        help = "Notes file to open (default: see gim config)"
    )]
    file: Option<PathBuf>,

    #[arg(long, conflicts_with_all = ["kill", "daemon"], help = "Edit in this process only, no daemon")]
    local: bool,

    #[arg(
        long,
        conflicts_with = "daemon",
        help = "Save and stop the daemon for FILE"
    )]
    kill: bool,

    #[arg(long, hide = true, requires = "file")]
    daemon: bool,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    #[command(about = "Print resolved notes and state paths")]
    Config,
}

impl Args {
    fn path(&self) -> PathBuf {
        self.file.clone().unwrap_or_else(file::default_notes_path)
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let Some(Cmd::Config) = args.command {
        print_config();
        return ExitCode::SUCCESS;
    }
    if args.daemon {
        let Some(path) = args.file.as_ref() else {
            return ExitCode::from(2);
        };
        return daemon(path);
    }
    let path = args.path();
    let result = if args.kill {
        kill(&path)
    } else if args.local {
        local(&path)
    } else {
        open(&path)
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("gim: {e}");
            ExitCode::from(1)
        }
    }
}

fn print_config() {
    println!("notes  {}", file::default_notes_path().display());
    match state_dir() {
        Ok(dir) => println!("state  {}", dir.display()),
        Err(e) => println!("state  ({e})"),
    }
}

#[cfg(unix)]
fn state_dir() -> io::Result<PathBuf> {
    gim::daemon::state_dir()
}

#[cfg(not(unix))]
fn state_dir() -> io::Result<PathBuf> {
    Err(io::Error::other(NO_DAEMONS))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_is_a_subcommand() {
        let args = Args::parse_from(["gim", "config"]);
        assert!(matches!(args.command, Some(Cmd::Config)));
        assert!(args.file.is_none());
    }

    #[test]
    fn file_is_optional_and_kill_is_a_flag() {
        let args = Args::parse_from(["gim", "--kill", "notes.md"]);
        assert!(args.kill);
        assert_eq!(args.file.as_deref(), Some(Path::new("notes.md")));
        assert!(args.command.is_none());
    }
}
