use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, MouseEventKind};
use gim::app::{App, Flow};
use gim::clipboard::default_clipboard;
use gim::file::{self, Document};
use gim::terminal::{self, TerminalGuard};
use gim::ui;

const USAGE: &str = "usage: gim [FILE]

Opens FILE, or the default notes file when omitted:
$GIM_NOTES, else $XDG_DATA_HOME/gim/notes.md, else ~/.local/share/gim/notes.md.";

enum Cli {
    Open(PathBuf),
    Help,
    Usage,
}

fn parse_args(args: &[OsString]) -> Cli {
    match args {
        [] => Cli::Open(file::default_notes_path()),
        [one] if one == "-h" || one == "--help" => Cli::Help,
        [one] if one.to_string_lossy().starts_with('-') => Cli::Usage,
        [one] => Cli::Open(PathBuf::from(one)),
        _ => Cli::Usage,
    }
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let path = match parse_args(&args) {
        Cli::Open(path) => path,
        Cli::Help => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Cli::Usage => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let (doc, text) = match file::load(&path) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("gim: {e}");
            return ExitCode::from(1);
        }
    };
    match run(doc, text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("gim: {e}");
            ExitCode::from(1)
        }
    }
}

/// Sets up the terminal and runs the event loop until the app quits. The
/// guard restores the terminal before this returns, so errors print cleanly.
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
        let timeout = app.next_deadline().unwrap_or(Duration::from_secs(3600));
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
