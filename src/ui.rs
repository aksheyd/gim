use std::ops::Range;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{App, HINT, Mode, QUIT_PROMPT};
use crate::buffer::Pos;
use crate::file::LineEnding;
use crate::text::{display_width, line_start, slice};
use crate::wrap::{Layout, TAB_STOP};

pub fn text_area(size: Rect) -> Rect {
    let height = if size.height >= 2 {
        size.height - 1
    } else {
        size.height
    };
    Rect::new(size.x, size.y, size.width, height)
}

fn reversed() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

pub struct TextView<'a> {
    pub text: &'a str,
    pub layout: &'a Layout,
    pub scroll: usize,
    pub selection: Option<Range<Pos>>,
}

fn display_row(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for g in segment.graphemes(true) {
        match g.chars().next() {
            Some('\t') => out.extend(std::iter::repeat_n(' ', TAB_STOP)),
            Some(c) if c.is_control() => out.push('?'),
            _ => out.push_str(g),
        }
    }
    out
}

impl Widget for TextView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let width = usize::from(area.width);
        for (i, y) in (area.top()..area.bottom()).enumerate() {
            let Some(row) = self.layout.rows().get(self.scroll + i) else {
                break;
            };
            let segment = slice(self.text, row.clone());
            buf.set_stringn(area.x, y, display_row(segment), width, Style::default());
            let Some(sel) = &self.selection else {
                continue;
            };
            let start = sel.start.max(row.start);
            let end = sel.end.min(row.end);
            if start >= end {
                continue;
            }
            let x0 = display_width(slice(self.text, row.start..start)).min(width);
            let x1 = display_width(slice(self.text, row.start..end)).min(width);
            if x1 > x0 {
                let cells = Rect::new(area.x + x0 as u16, y, (x1 - x0) as u16, 1);
                buf.set_style(cells, reversed());
            }
        }
    }
}

pub fn draw(app: &App, frame: &mut Frame) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let text = text_area(area);
    let status = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    if area.height == 1 && app.mode() == Mode::QuitPrompt {
        paint_status(app, status, frame.buffer_mut());
        return;
    }
    let editor = &app.editor;
    let viewport = editor.viewport();
    frame.render_widget(
        TextView {
            text: editor.text(),
            layout: editor.layout(),
            scroll: viewport.scroll,
            selection: editor.selection_range(),
        },
        text,
    );
    let (row, col) = editor.visual_cursor();
    let visible_rows = viewport.scroll..viewport.scroll + usize::from(text.height);
    if visible_rows.contains(&row) {
        let y = text.y + (row - viewport.scroll) as u16;
        frame.set_cursor_position((text.x + col as u16, y));
    }
    if area.height >= 2 {
        paint_status(app, status, frame.buffer_mut());
    }
}

pub fn cursor_label(text: &str, cursor: Pos) -> String {
    let line = slice(text, 0..cursor).matches('\n').count() + 1;
    let col = display_width(slice(text, line_start(text, cursor)..cursor)) + 1;
    format!("Ln {line}, Col {col}")
}

fn status_left(app: &App) -> String {
    let mut left = format!(" {}", app.doc.display_name);
    if app.is_dirty() {
        left.push_str(" [+]");
    }
    if !app.doc.exists {
        left.push_str(" (new)");
    }
    if app.doc.ending == LineEnding::Crlf {
        left.push_str(" CRLF");
    }
    if app.doc.bom {
        left.push_str(" BOM");
    }
    left
}

fn paint_status(app: &App, area: Rect, buf: &mut Buffer) {
    let style = reversed();
    let width = usize::from(area.width);
    for x in area.left()..area.right() {
        buf[(x, area.y)].set_symbol(" ").set_style(style);
    }
    let left = status_left(app);
    let mut centre = match (app.mode(), app.message()) {
        (Mode::QuitPrompt, Some(error)) => format!("{error} — {QUIT_PROMPT}"),
        (Mode::QuitPrompt, None) => QUIT_PROMPT.to_string(),
        (Mode::Normal, Some(message)) => message.to_string(),
        (Mode::Normal, None) => HINT.to_string(),
    };
    let right = cursor_label(app.editor.text(), app.editor.cursor());
    let lw = display_width(&left);
    let rw = display_width(&right);
    let hint_only = app.mode() == Mode::Normal && app.message().is_none();
    if hint_only && lw + 1 + display_width(&centre) + 1 + rw > width {
        centre.clear();
    }
    let cw = display_width(&centre);
    let y = area.y;
    if cw > 0 {
        if lw + 1 + cw + 1 + rw <= width {
            buf.set_stringn(area.x, y, &left, width, style);
            let cx = ((width - cw) / 2).clamp(lw + 1, width - rw - 1 - cw);
            buf.set_stringn(area.x + cx as u16, y, &centre, cw, style);
            buf.set_stringn(area.x + (width - rw) as u16, y, &right, rw, style);
        } else if lw + 1 + cw <= width {
            buf.set_stringn(area.x, y, &left, width, style);
            buf.set_stringn(area.x + (lw + 1) as u16, y, &centre, cw, style);
        } else {
            buf.set_stringn(area.x, y, &centre, width, style);
        }
    } else if lw + 1 + rw <= width {
        buf.set_stringn(area.x, y, &left, width, style);
        buf.set_stringn(area.x + (width - rw) as u16, y, &right, rw, style);
    } else {
        buf.set_stringn(area.x, y, &left, width, style);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Instant;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::clipboard::MemClipboard;
    use crate::file::Document;

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

    fn cursor(terminal: &mut Terminal<TestBackend>) -> Option<(u16, u16)> {
        if !terminal.backend().cursor_visible() {
            return None;
        }
        let pos = terminal.get_cursor_position().unwrap();
        Some((pos.x, pos.y))
    }

    fn key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
        let event = crate::Event::Key(KeyEvent::new(code, mods));
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
}
