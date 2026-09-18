use std::io::{self, Stdout};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    supports_keyboard_enhancement,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

#[derive(Debug, Default)]
pub struct TerminalState {
    active: AtomicBool,
    kitty: AtomicBool,
}

pub struct TerminalGuard {
    state: Arc<TerminalState>,
}

impl TerminalGuard {
    pub fn enter() -> io::Result<(Self, Terminal<CrosstermBackend<Stdout>>)> {
        enable_raw_mode()?;
        let state = Arc::new(TerminalState::default());
        state.active.store(true, Ordering::SeqCst);
        let guard = TerminalGuard {
            state: Arc::clone(&state),
        };
        let kitty = supports_keyboard_enhancement().unwrap_or(false);
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;
        if kitty {
            let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
            execute!(out, PushKeyboardEnhancementFlags(flags))?;
            state.kitty.store(true, Ordering::SeqCst);
        }
        execute!(out, EnableBracketedPaste, EnableMouseCapture)?;
        let terminal = Terminal::new(CrosstermBackend::new(out))?;
        Ok((guard, terminal))
    }

    pub fn state(&self) -> Arc<TerminalState> {
        Arc::clone(&self.state)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore(&self.state);
    }
}

pub fn restore(state: &TerminalState) {
    if !state.active.swap(false, Ordering::SeqCst) {
        return;
    }
    let kitty = state.kitty.swap(false, Ordering::SeqCst);
    let mut out = io::stdout();
    let _ = execute!(out, DisableMouseCapture, DisableBracketedPaste);
    if kitty {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = execute!(out, LeaveAlternateScreen);
    if kitty {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ = disable_raw_mode();
}

pub fn install_panic_hook(state: Arc<TerminalState>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore(&state);
        previous(info);
    }));
}
