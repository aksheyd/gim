mod common;
use common::temp_dir;
use std::fs;
use std::time::Instant;

use gim::app::{AUTOSAVE_DELAY, App, Flow, Mode};
use gim::clipboard::MemClipboard;
use gim::file::Document;
use gim::keys::Action;
use gim::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;

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

#[test]
fn save_messages_and_dirty_across_undo() {
    let mut app = make_app("");
    assert!(!app.is_dirty());
    app.editor.handle(Action::Recenter);
    assert_eq!(app.editor.pinned_scroll(), Some(0));
    ctrl(&mut app, 's');
    assert_eq!(app.editor.pinned_scroll(), None);
    assert!(app.message().unwrap().starts_with("Saved 0 bytes"));
    assert!(app.doc.exists);
    ctrl(&mut app, 's');
    assert_eq!(app.message(), Some("no changes"));
    plain(&mut app, 'a');
    assert!(app.is_dirty());
    assert_eq!(app.message(), None);
    ctrl(&mut app, 'z');
    assert!(!app.is_dirty());
    key(&mut app, KeyCode::Char('Z'), KeyModifiers::CONTROL);
    assert!(app.is_dirty());
    ctrl(&mut app, 's');
    assert_eq!(app.message(), Some("Saved 1 byte"));
    assert!(!app.is_dirty());
    cleanup(&app);
}

#[test]
fn ctrl_c_copies_only_a_selection_and_paste_normalises() {
    let mut app = make_app("ab");
    ctrl(&mut app, 'c');
    assert_eq!(app.editor.text(), "ab");
    key(&mut app, KeyCode::Right, KeyModifiers::SHIFT);
    ctrl(&mut app, 'c');
    assert_eq!(app.message(), None);
    key(&mut app, KeyCode::End, KeyModifiers::NONE);
    ctrl(&mut app, 'v');
    assert_eq!(app.editor.text(), "aba");
    let paste = Event::Paste("x\r\ny\rz\u{2028}\t".to_string());
    app.handle_event(paste, Instant::now());
    assert_eq!(app.editor.text(), "abax\ny\nz\n\t");
    assert_eq!(app.editor.undo_len(), 2);
    cleanup(&app);
}

#[test]
fn resize_and_release_keys() {
    let mut app = make_app("hello world");
    app.handle_event(Event::Resize(5, 3), Instant::now());
    assert_eq!(app.editor.viewport().width, 5);
    assert_eq!(app.editor.viewport().height, 2);
    assert_eq!(app.editor.layout().row_count(), 2);
    app.handle_event(Event::Resize(5, 1), Instant::now());
    assert_eq!(app.editor.viewport().height, 1);
    app.handle_event(Event::Resize(0, 0), Instant::now());
    assert_eq!(app.text_rect(), Rect::new(0, 0, 0, 0));
    let kind = KeyEventKind::Release;
    let release = KeyEvent::new_with_kind(KeyCode::Char('x'), KeyModifiers::NONE, kind);
    app.handle_event(Event::Key(release), Instant::now());
    assert_eq!(app.editor.text(), "hello world");
    cleanup(&app);
}
