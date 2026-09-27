use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

use crate::clipboard::Clipboard;
use crate::editor::{Editor, Effect};
use crate::file::{self, Document};
use crate::keys::{Action, classify};
use crate::text::normalize_newlines;
use crate::ui::text_area;

pub const QUIT_PROMPT: &str = "y: retry  n: quit without saving  Esc: keep editing";
pub const HINT: &str = "Ctrl-Q to quit";
pub const AUTOSAVE_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    QuitPrompt,
}

pub struct App {
    pub editor: Editor,
    pub doc: Document,
    mode: Mode,
    message: Option<String>,
    clipboard: Box<dyn Clipboard>,
    size: (u16, u16),
    autosave_at: Option<Instant>,
}

impl App {
    pub fn new(
        doc: Document,
        text: String,
        clipboard: Box<dyn Clipboard>,
        width: u16,
        height: u16,
    ) -> Self {
        let area = text_area(Rect::new(0, 0, width, height));
        App {
            editor: Editor::new(text, usize::from(area.width), usize::from(area.height)),
            doc,
            mode: Mode::Normal,
            message: None,
            clipboard,
            size: (width, height),
            autosave_at: None,
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    pub fn is_dirty(&self) -> bool {
        self.doc.is_dirty(self.editor.text())
    }

    pub fn text_rect(&self) -> Rect {
        text_area(Rect::new(0, 0, self.size.0, self.size.1))
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.size = (width, height);
        let area = self.text_rect();
        let (width, height) = (usize::from(area.width), usize::from(area.height));
        self.editor.set_viewport(width, height);
    }

    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        let autosave = self.autosave_at.map(|at| at.saturating_duration_since(now));
        match (self.editor.next_deadline(), autosave) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn advance(&mut self, now: Instant) -> bool {
        let effect = self.editor.advance(now);
        let mut changed = effect != Effect::Nothing;
        self.apply_effect(effect);
        if self.autosave_at.is_some_and(|at| at <= now) {
            self.autosave_at = None;
            if let Err(message) = self.save() {
                self.message = Some(message);
            }
            changed = true;
        }
        changed
    }

    pub fn handle_event(&mut self, event: Event, now: Instant) -> Flow {
        let before = self.editor.mutations();
        let flow = self.dispatch_event(event, now);
        if self.editor.mutations() != before {
            self.autosave_at = Some(now + AUTOSAVE_DELAY);
        }
        if !self.is_dirty() {
            self.autosave_at = None;
        }
        if flow == Flow::Quit {
            self.mode = Mode::Normal;
            self.message = None;
        }
        flow
    }

    fn dispatch_event(&mut self, event: Event, now: Instant) -> Flow {
        match event {
            Event::Key(key) => {
                if key.kind == KeyEventKind::Release {
                    return Flow::Continue;
                }
                self.editor.clear_pin();
                if self.mode == Mode::QuitPrompt {
                    return self.prompt_key(key);
                }
                self.message = None;
                match classify(key) {
                    Some(action) => self.action(action),
                    None => Flow::Continue,
                }
            }
            Event::Paste(text) => {
                if self.mode == Mode::Normal {
                    let text = normalize_newlines(&text);
                    let effect = self.editor.insert_text(&text);
                    self.apply_effect(effect);
                }
                Flow::Continue
            }
            Event::Mouse(mouse) => {
                if self.mode == Mode::Normal {
                    let area = self.text_rect();
                    let effect = self.editor.mouse(mouse, area, now);
                    self.apply_effect(effect);
                }
                Flow::Continue
            }
            Event::Resize(width, height) => {
                self.resize(width, height);
                Flow::Continue
            }
            Event::FocusGained | Event::FocusLost => Flow::Continue,
        }
    }

    fn prompt_key(&mut self, key: KeyEvent) -> Flow {
        let chords = KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER;
        let chord = key.modifiers.intersects(chords);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('y' | 'Y') if !chord => self.save_and_quit(),
            KeyCode::Char('n' | 'N') if !chord => Flow::Quit,
            KeyCode::Esc => self.cancel_prompt(),
            KeyCode::Char('q' | 'c') if ctrl => self.cancel_prompt(),
            _ => Flow::Continue,
        }
    }

    fn cancel_prompt(&mut self) -> Flow {
        self.mode = Mode::Normal;
        self.message = None;
        Flow::Continue
    }

    fn save_and_quit(&mut self) -> Flow {
        match self.save() {
            Ok(_) => Flow::Quit,
            Err(message) => {
                self.editor.end_drag();
                self.mode = Mode::QuitPrompt;
                self.message = Some(message);
                Flow::Continue
            }
        }
    }

    fn action(&mut self, action: Action) -> Flow {
        match action {
            Action::Save => {
                let message = match self.save() {
                    Ok(m) | Err(m) => m,
                };
                self.message = Some(message);
            }
            Action::Quit => return self.save_and_quit(),
            Action::Paste => {
                if let Some(text) = self.clipboard.get() {
                    let text = normalize_newlines(&text);
                    let effect = self.editor.insert_text(&text);
                    self.apply_effect(effect);
                }
            }
            other => {
                let effect = self.editor.handle(other);
                self.apply_effect(effect);
            }
        }
        Flow::Continue
    }

    fn apply_effect(&mut self, effect: Effect) {
        if let Effect::Copy(text) = effect {
            self.clipboard.set(&text);
        }
    }

    pub fn save(&mut self) -> Result<String, String> {
        if self.doc.exists && !self.is_dirty() {
            return Ok("no changes".to_string());
        }
        let outcome = match file::save(&mut self.doc, self.editor.text()) {
            Ok(outcome) => outcome,
            Err(e) => return Err(e.to_string()),
        };
        if outcome.atomic {
            Ok(format!("Saved {}", plural(outcome.bytes, "byte")))
        } else {
            Ok("saved (non-atomic)".to_string())
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        format!("{n} {word}")
    } else {
        format!("{n} {word}s")
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::clipboard::MemClipboard;
    use crate::file::temp_dir;

    fn make_app(text: &str) -> App {
        let doc = Document::new(&temp_dir().join("notes.md"));
        let clipboard = Box::new(MemClipboard::default());
        App::new(doc, text.to_string(), clipboard, 40, 10)
    }

    fn key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Flow {
        app.handle_event(Event::Key(KeyEvent::new(code, mods)), Instant::now())
    }

    fn ctrl(app: &mut App, c: char) -> Flow {
        key(app, KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn plain(app: &mut App, c: char) -> Flow {
        key(app, KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn cleanup(app: &App) {
        if let Some(dir) = app.doc.path.parent() {
            let _ = fs::remove_dir_all(dir);
        }
    }

    fn block_saves(app: &mut App) {
        let blocker = app.doc.path.parent().unwrap().join("blocker");
        fs::write(&blocker, "").unwrap();
        app.doc.path = blocker.join("child.md");
    }

    #[test]
    fn quit_saves_first_and_only_prompts_when_the_save_fails() {
        let mut app = make_app("");
        assert_eq!(ctrl(&mut app, 'q'), Flow::Quit);
        plain(&mut app, 'x');
        assert_eq!(ctrl(&mut app, 'q'), Flow::Quit);
        assert_eq!(fs::read_to_string(&app.doc.path).unwrap(), "x");
        cleanup(&app);

        let mut app = make_app("");
        block_saves(&mut app);
        plain(&mut app, 'x');
        assert_eq!(ctrl(&mut app, 'q'), Flow::Continue);
        assert_eq!(app.mode(), Mode::QuitPrompt);
        let error = app.message().unwrap().to_string();
        assert_eq!(plain(&mut app, 'z'), Flow::Continue);
        assert_eq!(app.mode(), Mode::QuitPrompt);
        assert_eq!(app.message(), Some(error.as_str()));
        assert_eq!(app.editor.text(), "x");
        let paste = Event::Paste("ignored".to_string());
        assert_eq!(app.handle_event(paste, Instant::now()), Flow::Continue);
        assert_eq!(app.editor.text(), "x");
        assert_eq!(plain(&mut app, 'y'), Flow::Continue);
        assert_eq!(app.mode(), Mode::QuitPrompt);
        assert_eq!(
            key(&mut app, KeyCode::Esc, KeyModifiers::NONE),
            Flow::Continue
        );
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.message(), None);
        ctrl(&mut app, 'q');
        assert_eq!(ctrl(&mut app, 'q'), Flow::Continue);
        assert_eq!(app.mode(), Mode::Normal);
        ctrl(&mut app, 'q');
        assert_eq!(ctrl(&mut app, 'c'), Flow::Continue);
        assert_eq!(app.mode(), Mode::Normal);
        ctrl(&mut app, 'q');
        assert_eq!(plain(&mut app, 'n'), Flow::Quit);
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.message(), None);
        assert!(!app.doc.path.exists());
        cleanup(&app);

        let mut app = make_app("");
        block_saves(&mut app);
        plain(&mut app, 'x');
        assert_eq!(ctrl(&mut app, 'q'), Flow::Continue);
        assert_eq!(app.mode(), Mode::QuitPrompt);
        let blocker = app.doc.path.parent().unwrap().to_path_buf();
        fs::remove_file(&blocker).unwrap();
        assert_eq!(plain(&mut app, 'y'), Flow::Quit);
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.message(), None);
        assert_eq!(fs::read_to_string(&app.doc.path).unwrap(), "x");
        cleanup(&app);
    }

    #[test]
    fn autosave_fires_after_idle_and_reports_failures_once() {
        let mut app = make_app("");
        let t = Instant::now();
        assert_eq!(app.next_deadline(t), None);
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            t,
        );
        assert_eq!(app.next_deadline(t), Some(AUTOSAVE_DELAY));
        let later = t + AUTOSAVE_DELAY / 2;
        let right = Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        app.handle_event(right, later);
        assert_eq!(app.next_deadline(later), Some(AUTOSAVE_DELAY / 2));
        let typed = Event::Key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        app.handle_event(typed, later);
        assert_eq!(app.next_deadline(later), Some(AUTOSAVE_DELAY));
        assert!(!app.advance(later + AUTOSAVE_DELAY / 2));
        assert!(app.is_dirty());
        assert!(app.advance(later + AUTOSAVE_DELAY));
        assert!(!app.is_dirty());
        assert_eq!(fs::read_to_string(&app.doc.path).unwrap(), "ba");
        assert_eq!(app.message(), None);
        assert_eq!(app.next_deadline(later + AUTOSAVE_DELAY), None);
        ctrl(&mut app, 'z');
        assert!(app.is_dirty());
        assert!(app.next_deadline(Instant::now()).is_some());
        cleanup(&app);

        let mut app = make_app("");
        block_saves(&mut app);
        app.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
            t,
        );
        assert!(app.advance(t + AUTOSAVE_DELAY));
        assert!(app.message().is_some());
        assert!(app.is_dirty());
        assert_eq!(app.next_deadline(t + AUTOSAVE_DELAY), None);
        cleanup(&app);
    }
}
