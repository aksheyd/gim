use gim::app::{App, Mode, QUIT_PROMPT};
use gim::clipboard::MemClipboard;
use gim::file::{Document, LineEnding};
use gim::ui::{cursor_label, draw};
use gim::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use std::path::Path;
use std::time::Instant;

fn make_app(text: &str, width: u16, height: u16) -> App {
    let mut doc = Document::new(Path::new("notes.md"));
    doc.saved_text = text.to_string();
    let clipboard = Box::new(MemClipboard::default());
    App::new(doc, text.to_string(), clipboard, width, height)
}

fn render(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| draw(app, frame)).unwrap();
    terminal
}

fn row(terminal: &Terminal<TestBackend>, y: u16) -> String {
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    for x in 0..buf.area.width {
        out.push_str(buf[(x, y)].symbol());
    }
    out
}

fn reversed_at(terminal: &Terminal<TestBackend>, x: u16, y: u16) -> bool {
    let style = terminal.backend().buffer()[(x, y)].style();
    style.add_modifier.contains(Modifier::REVERSED)
}

fn cursor(terminal: &mut Terminal<TestBackend>) -> Option<(u16, u16)> {
    if !terminal.backend().cursor_visible() {
        return None;
    }
    let pos = terminal.get_cursor_position().unwrap();
    Some((pos.x, pos.y))
}

fn key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let event = gim::Event::Key(KeyEvent::new(code, mods));
    app.handle_event(event, Instant::now());
}

#[test]
fn wraps_expands_tabs_and_marks_controls() {
    let mut app = make_app("hello world\n\tx\u{7}y", 30, 5);
    key(&mut app, KeyCode::End, KeyModifiers::NONE);
    let mut t = render(&app, 30, 5);
    assert_eq!(row(&t, 0), "hello world                   ");
    assert_eq!(row(&t, 1), "    x?y                       ");
    assert_eq!(row(&t, 2), "                              ");
    assert_eq!(row(&t, 4), " notes.md (new)   Ln 1, Col 12");
    assert_eq!(cursor(&mut t), Some((11, 0)));
    let app = make_app("hello world", 5, 5);
    let mut t = render(&app, 5, 5);
    assert_eq!(row(&t, 0), "hello");
    assert_eq!(row(&t, 1), "world");
    assert_eq!(cursor(&mut t), Some((0, 0)));
}

#[test]
fn cursor_at_soft_wrap_boundary_and_phantom_row() {
    let mut app = make_app("hello world", 5, 5);
    for _ in 0..5 {
        key(&mut app, KeyCode::Right, KeyModifiers::NONE);
    }
    let mut t = render(&app, 5, 5);
    assert_eq!(cursor(&mut t), Some((0, 1)));
    assert_eq!(row(&t, 4), " note");
    key(&mut app, KeyCode::End, KeyModifiers::NONE);
    let mut t = render(&app, 5, 5);
    assert_eq!(cursor(&mut t), Some((0, 2)));
    let mut app = make_app("abcde\nfghij\nklmno", 5, 3);
    key(&mut app, KeyCode::End, KeyModifiers::CONTROL);
    let mut t = render(&app, 5, 3);
    assert_eq!(row(&t, 0), "klmno");
    assert_eq!(row(&t, 1), "     ");
    assert_eq!(cursor(&mut t), Some((0, 1)));
}

#[test]
fn pinned_scroll_hides_cursor_and_typing_restores_it() {
    let mut app = make_app("a\nb\nc\nd\ne\nf\ng", 20, 4);
    let mut t = render(&app, 20, 4);
    assert_eq!(cursor(&mut t), Some((0, 0)));
    let wheel = gim::MouseEvent {
        kind: gim::MouseEventKind::ScrollDown,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_event(gim::Event::Mouse(wheel), Instant::now());
    let mut t = render(&app, 20, 4);
    assert_eq!(row(&t, 0), "b                   ");
    assert_eq!(cursor(&mut t), None);
    key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
    let mut t = render(&app, 20, 4);
    assert_eq!(row(&t, 0), "za                  ");
    assert_eq!(cursor(&mut t), Some((1, 0)));
    assert_eq!(row(&t, 3), " notes.md [+] (new) ");
}

#[test]
fn selection_is_reversed_and_status_shows_message_and_markers() {
    let mut app = make_app("abc def", 60, 3);
    key(&mut app, KeyCode::Right, KeyModifiers::SHIFT);
    key(&mut app, KeyCode::Right, KeyModifiers::SHIFT);
    let t = render(&app, 60, 3);
    assert!(reversed_at(&t, 0, 0));
    assert!(reversed_at(&t, 1, 0));
    assert!(!reversed_at(&t, 2, 0));
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    let t = render(&app, 60, 3);
    assert_eq!(
        row(&t, 2),
        " notes.md (new)        Ctrl-Q to quit            Ln 1, Col 3"
    );
    let mut app = make_app("x", 12, 2);
    app.doc.ending = LineEnding::Crlf;
    app.doc.bom = true;
    let t = render(&app, 12, 2);
    assert_eq!(row(&t, 1), " notes.md (n");
}

#[test]
fn degenerate_sizes() {
    let mut app = make_app("abc", 1, 1);
    key(&mut app, KeyCode::Right, KeyModifiers::NONE);
    let mut t = render(&app, 1, 1);
    assert_eq!(row(&t, 0), "b");
    assert_eq!(cursor(&mut t), Some((0, 0)));
    let mut app = make_app("hello", 10, 1);
    let dir = std::env::temp_dir().join(format!("gim-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, "").unwrap();
    app.doc.path = blocker.join("child.md");
    key(&mut app, KeyCode::Char('!'), KeyModifiers::NONE);
    let mut t = render(&app, 10, 1);
    assert_eq!(row(&t, 0), "!hello    ");
    assert_eq!(cursor(&mut t), Some((1, 0)));
    key(&mut app, KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert_eq!(app.mode(), Mode::QuitPrompt);
    let prompt = format!("{} — {QUIT_PROMPT}", app.message().unwrap());
    let mut t = render(&app, 10, 1);
    assert_eq!(row(&t, 0), prompt.chars().take(10).collect::<String>());
    assert_eq!(cursor(&mut t), None);
    let _ = std::fs::remove_dir_all(blocker.parent().unwrap());
    let app = make_app("", 0, 0);
    let mut t = render(&app, 0, 0);
    assert_eq!(cursor(&mut t), None);
    assert_eq!(cursor_label("ab\n한\tc", 8), "Ln 2, Col 8");
}
