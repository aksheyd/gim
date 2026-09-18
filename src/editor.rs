use std::ops::Range;

use crate::buffer::{Buffer, Edit, EditKind, Pos};
use crate::keys::{Action, DeleteKind, Motion};
use crate::mouse::MouseState;
use crate::text::{
    WordKind, ceil_boundary, floor_boundary, line_end, line_start, next_boundary, prev_boundary,
    slice, word_left, word_right,
};
use crate::undo::History;
use crate::wrap::{Layout, wrap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    pub width: usize,
    pub height: usize,
    pub scroll: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
}

impl Selection {
    pub fn new(anchor: Pos, head: Pos) -> Self {
        Selection { anchor, head }
    }

    pub fn range(&self) -> Range<Pos> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Nothing,
    Redraw,
    Copy(String),
}

#[derive(Debug)]
pub struct Editor {
    buf: Buffer,
    history: History,
    selection: Option<Selection>,
    kill: Option<String>,
    sticky_col: Option<usize>,
    layout: Layout,
    layout_stale: bool,
    viewport: Viewport,
    pinned_scroll: Option<usize>,
    mutations: u64,
    pub(crate) mouse: MouseState,
}

pub fn resolve_scroll(
    total: usize,
    cursor_row: usize,
    prev: usize,
    pinned: Option<usize>,
    height: usize,
) -> usize {
    let height = height.max(1);
    if total <= height {
        return 0;
    }
    let max_scroll = total - height;
    if let Some(p) = pinned {
        return p.min(max_scroll);
    }
    let scroll = prev.min(max_scroll);
    if cursor_row < scroll {
        cursor_row
    } else if cursor_row >= scroll + height {
        cursor_row + 1 - height
    } else {
        scroll
    }
}

fn horizontal_target(text: &str, pos: Pos, motion: Motion) -> Pos {
    let len = text.len();
    match motion {
        Motion::Left => prev_boundary(text, pos),
        Motion::Right => next_boundary(text, pos),
        Motion::WordLeft => word_left(text, pos, WordKind::Word),
        Motion::WordRight => word_right(text, pos, WordKind::Word),
        Motion::LineStart => line_start(text, pos),
        Motion::LineEnd => line_end(text, pos),
        Motion::LineStartChain => {
            let start = line_start(text, pos);
            if start == pos && pos > 0 {
                line_start(text, pos - 1)
            } else {
                start
            }
        }
        Motion::LineEndChain => {
            let end = line_end(text, pos);
            if end == pos && pos < len {
                line_end(text, pos + 1)
            } else {
                end
            }
        }
        Motion::DocStart => 0,
        Motion::DocEnd => len,
        Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown => pos,
    }
}

impl Editor {
    pub fn new(text: String, width: usize, height: usize) -> Self {
        let layout = wrap(&text, width);
        Editor {
            buf: Buffer::new(text),
            history: History::default(),
            selection: None,
            kill: None,
            sticky_col: None,
            layout,
            layout_stale: false,
            mutations: 0,
            viewport: Viewport {
                width,
                height,
                scroll: 0,
            },
            pinned_scroll: None,
            mouse: MouseState::default(),
        }
    }

    pub fn text(&self) -> &str {
        self.buf.text()
    }

    pub fn cursor(&self) -> Pos {
        self.buf.cursor()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }

    pub fn selection_range(&self) -> Option<Range<Pos>> {
        self.selection.map(|s| s.range())
    }

    pub fn kill(&self) -> Option<&str> {
        self.kill.as_deref()
    }

    pub fn sticky_col(&self) -> Option<usize> {
        self.sticky_col
    }

    pub fn pinned_scroll(&self) -> Option<usize> {
        self.pinned_scroll
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn undo_len(&self) -> usize {
        self.history.undo_len()
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn set_viewport(&mut self, width: usize, height: usize) {
        self.viewport.width = width;
        self.viewport.height = height;
        self.clamp_mouse_anchor();
        self.ensure_layout();
    }

    pub fn ensure_layout(&mut self) {
        let width = self.viewport.width.max(1);
        if self.layout_stale || self.layout.width() != width {
            self.layout = wrap(self.buf.text(), width);
            self.layout_stale = false;
        }
        let (row, _) = self.visual_cursor();
        let total = self.total_rows();
        self.viewport.scroll = resolve_scroll(
            total,
            row,
            self.viewport.scroll,
            self.pinned_scroll,
            self.viewport.height,
        );
    }

    pub fn visual_cursor(&self) -> (usize, usize) {
        self.layout.visual_pos(self.buf.text(), self.buf.cursor())
    }

    pub fn total_rows(&self) -> usize {
        let (row, _) = self.visual_cursor();
        self.layout.row_count().max(row + 1)
    }

    pub fn handle(&mut self, action: Action) -> Effect {
        let effect = self.dispatch(action);
        self.ensure_layout();
        effect
    }

    pub fn insert_text(&mut self, text: &str) -> Effect {
        let at = self.cursor();
        let range = self.selection_range().unwrap_or(at..at);
        self.edit(range, text.to_string(), EditKind::Replace);
        self.ensure_layout();
        Effect::Redraw
    }

    pub(crate) fn clear_pin(&mut self) {
        self.pinned_scroll = None;
        self.ensure_layout();
    }

    fn undo(&mut self) {
        if self.history.undo(&mut self.buf) {
            self.after_mutation();
        }
    }

    fn redo(&mut self) {
        if self.history.redo(&mut self.buf) {
            self.after_mutation();
        }
    }

    fn dispatch(&mut self, action: Action) -> Effect {
        if action != Action::Recenter {
            self.pinned_scroll = None;
        }
        let selected = self.selection.is_some();
        let at = self.cursor();
        let range = self.selection_range().unwrap_or(at..at);
        match action {
            Action::Select(motion) => self.extend_selection(motion),
            Action::Move(motion) if selected => self.collapse_then_move(range, motion),
            Action::Move(motion) => self.apply_motion(motion),
            Action::Insert(c) => self.edit(range, c.to_string(), EditKind::Insert),
            Action::Newline => self.edit(range, "\n".to_string(), EditKind::Insert),
            Action::Delete(kind) if selected => {
                let kind = if kind.is_kill() {
                    EditKind::Kill
                } else {
                    EditKind::Delete
                };
                self.edit(range, String::new(), kind);
            }
            Action::Delete(kind) => self.delete(kind),
            Action::Yank => {
                if let Some(kill) = self.kill.clone() {
                    self.edit(range, kill, EditKind::Replace);
                }
            }
            Action::Copy if selected => {
                return Effect::Copy(slice(self.buf.text(), range).to_string());
            }
            Action::Cut if selected => {
                let cut = slice(self.buf.text(), range.clone()).to_string();
                self.edit(range, String::new(), EditKind::Replace);
                return Effect::Copy(cut);
            }
            Action::Escape => self.selection = None,
            _ => {
                self.selection = None;
                match action {
                    Action::Undo => self.undo(),
                    Action::Redo => self.redo(),
                    Action::SelectAll => self.select_all(),
                    Action::Recenter => {
                        let (row, _) = self.visual_cursor();
                        self.pinned_scroll = Some(row.saturating_sub(self.viewport.height / 2));
                    }
                    _ => return Effect::Nothing,
                }
            }
        }
        Effect::Redraw
    }

    fn select_all(&mut self) {
        if !self.is_empty() {
            let len = self.len();
            self.selection = Some(Selection::new(0, len));
            self.buf.set_cursor(len);
            self.sticky_col = None;
        }
    }

    fn edit(&mut self, range: Range<Pos>, text: String, kind: EditKind) {
        let start = floor_boundary(self.buf.text(), range.start);
        let end = ceil_boundary(self.buf.text(), range.end).max(start);
        let edit = Edit {
            range: start..end,
            cursor: start + text.len(),
            text,
            kind,
        };
        let Some(delta) = self.buf.apply(edit) else {
            return;
        };
        if delta.kind == EditKind::Kill && !delta.removed.is_empty() {
            self.kill = Some(delta.removed.clone());
        }
        self.history.record(delta);
        self.after_mutation();
    }

    fn after_mutation(&mut self) {
        self.selection = None;
        self.sticky_col = None;
        self.pinned_scroll = None;
        self.layout_stale = true;
        self.mutations += 1;
        self.clamp_mouse_anchor();
    }

    pub fn mutations(&self) -> u64 {
        self.mutations
    }

    pub(crate) fn set_selection(&mut self, selection: Option<Selection>) {
        self.selection = selection;
    }

    pub(crate) fn set_cursor(&mut self, pos: Pos) {
        self.buf.set_cursor(pos);
    }

    pub(crate) fn clear_sticky(&mut self) {
        self.sticky_col = None;
    }

    pub(crate) fn clear_sticky_and_pin(&mut self) {
        self.sticky_col = None;
        self.pinned_scroll = None;
    }

    pub(crate) fn pin_scroll(&mut self, scroll: usize) {
        self.pinned_scroll = Some(scroll);
        self.viewport.scroll = scroll;
    }

    fn clamp_mouse_anchor(&mut self) {
        if let Some(anchor) = self.mouse.anchor {
            self.mouse.anchor = Some(floor_boundary(self.buf.text(), anchor));
        }
    }

    fn delete(&mut self, kind: DeleteKind) {
        let text = self.buf.text();
        let pos = self.buf.cursor();
        let len = text.len();
        let (range, edit_kind) = match kind {
            DeleteKind::GraphemeBack => (prev_boundary(text, pos)..pos, EditKind::Delete),
            DeleteKind::GraphemeFwd => (pos..next_boundary(text, pos), EditKind::Delete),
            DeleteKind::WordBack(w) => (word_left(text, pos, w)..pos, EditKind::Kill),
            DeleteKind::WordFwd(w) => (pos..word_right(text, pos, w), EditKind::Kill),
            DeleteKind::ToLineStart => {
                let start = line_start(text, pos);
                if start < pos {
                    (start..pos, EditKind::Kill)
                } else if pos > 0 {
                    (pos - 1..pos, EditKind::Kill)
                } else {
                    return;
                }
            }
            DeleteKind::ToLineEnd => {
                let end = line_end(text, pos);
                if end > pos {
                    (pos..end, EditKind::Kill)
                } else if pos < len {
                    (pos..pos + 1, EditKind::Kill)
                } else {
                    return;
                }
            }
        };
        self.edit(range, String::new(), edit_kind);
    }

    fn extend_selection(&mut self, motion: Motion) {
        let anchor = self.selection.map_or(self.cursor(), |s| s.anchor);
        if let Some(sel) = self.selection {
            self.buf.set_cursor(sel.head);
        }
        self.apply_motion(motion);
        let head = self.cursor();
        self.selection = if head == anchor {
            None
        } else {
            Some(Selection::new(anchor, head))
        };
    }

    fn collapse_then_move(&mut self, range: Range<Pos>, motion: Motion) {
        self.selection = None;
        let to_start = matches!(
            motion,
            Motion::Left
                | Motion::WordLeft
                | Motion::LineStart
                | Motion::LineStartChain
                | Motion::Up
                | Motion::PageUp
                | Motion::DocStart
        );
        let edge = if to_start { range.start } else { range.end };
        self.buf.set_cursor(edge);
        match motion {
            Motion::Left | Motion::Right => self.sticky_col = None,
            _ => self.apply_motion(motion),
        }
    }

    fn apply_motion(&mut self, motion: Motion) {
        let step = self.page_step();
        let delta = match motion {
            Motion::Up => -1,
            Motion::Down => 1,
            Motion::PageUp => -step,
            Motion::PageDown => step,
            _ => {
                let target = horizontal_target(self.buf.text(), self.buf.cursor(), motion);
                self.buf.set_cursor(target);
                self.sticky_col = None;
                return;
            }
        };
        self.move_rows(delta);
    }

    fn page_step(&self) -> isize {
        self.viewport.height.saturating_sub(1).max(1) as isize
    }

    fn move_rows(&mut self, delta: isize) {
        let layout = &self.layout;
        let text = self.buf.text();
        let pos = self.buf.cursor();
        let row = layout.row_of(pos);
        let last = layout.row_count() - 1;
        let (target, sticky) = if delta < 0 && row == 0 {
            (0, None)
        } else if delta > 0 && row == last {
            (text.len(), None)
        } else {
            let col = self.sticky_col.unwrap_or_else(|| layout.col_of(text, pos));
            let target_row = row.saturating_add_signed(delta).min(last);
            (layout.pos_at(text, target_row, col), Some(col))
        };
        self.buf.set_cursor(target);
        self.sticky_col = sticky;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Motion::*;
    use crate::text::is_boundary;

    fn editor(text: &str, width: usize, height: usize) -> Editor {
        Editor::new(text.to_string(), width, height)
    }

    fn insert_action(c: char) -> Action {
        if c == '\n' {
            Action::Newline
        } else {
            Action::Insert(c)
        }
    }

    fn type_str(e: &mut Editor, s: &str) {
        for c in s.chars() {
            e.handle(insert_action(c));
        }
    }

    fn mv(e: &mut Editor, m: Motion) {
        e.handle(Action::Move(m));
    }

    fn goto(e: &mut Editor, pos: Pos) {
        e.buf.set_cursor(pos);
    }

    #[test]
    fn line_start_end_chain_with_edge_guards() {
        let mut e = editor("ab\ncd\nef", 80, 10);
        goto(&mut e, 4);
        mv(&mut e, LineStartChain);
        assert_eq!(e.cursor(), 3);
        mv(&mut e, LineStartChain);
        assert_eq!(e.cursor(), 0);
        mv(&mut e, LineStartChain);
        assert_eq!(e.cursor(), 0);
        mv(&mut e, LineEndChain);
        assert_eq!(e.cursor(), 2);
        mv(&mut e, LineEndChain);
        assert_eq!(e.cursor(), 5);
        mv(&mut e, LineEndChain);
        assert_eq!(e.cursor(), 8);
        mv(&mut e, LineEndChain);
        assert_eq!(e.cursor(), 8);
        goto(&mut e, 4);
        mv(&mut e, LineStart);
        assert_eq!(e.cursor(), 3);
        mv(&mut e, LineStart);
        assert_eq!(e.cursor(), 3);
        mv(&mut e, LineEnd);
        assert_eq!(e.cursor(), 5);
        mv(&mut e, LineEnd);
        assert_eq!(e.cursor(), 5);
    }

    #[test]
    fn kill_line_edges_and_yank() {
        let mut e = editor("ab\ncd", 80, 10);
        goto(&mut e, 3);
        e.handle(Action::Delete(DeleteKind::ToLineStart));
        assert_eq!(e.text(), "abcd");
        assert_eq!(e.kill(), Some("\n"));
        e.handle(Action::Delete(DeleteKind::ToLineEnd));
        assert_eq!(e.text(), "ab");
        assert_eq!(e.kill(), Some("cd"));
        assert_eq!(e.undo_len(), 2);
        goto(&mut e, 0);
        e.handle(Action::Yank);
        assert_eq!(e.text(), "cdab");
        assert_eq!(e.cursor(), 2);
        e.handle(Action::Undo);
        assert_eq!(e.text(), "ab");
        assert_eq!(e.kill(), Some("cd"));
        e.handle(Action::Redo);
        assert_eq!(e.text(), "cdab");
        assert_eq!(e.kill(), Some("cd"));
    }

    #[test]
    fn word_deletes_fill_kill_and_grapheme_deletes_do_not() {
        let mut e = editor("foo.bar   ", 80, 10);
        goto(&mut e, 10);
        e.handle(Action::Delete(DeleteKind::WordBack(WordKind::Word)));
        assert_eq!(e.text(), "foo.");
        assert_eq!(e.kill(), Some("bar   "));
        e.handle(Action::Delete(DeleteKind::GraphemeBack));
        assert_eq!(e.text(), "foo");
        assert_eq!(e.kill(), Some("bar   "));
        goto(&mut e, 0);
        e.handle(Action::Delete(DeleteKind::WordFwd(WordKind::BigWord)));
        assert_eq!(e.text(), "");
        assert_eq!(e.kill(), Some("foo"));
    }

    #[test]
    fn vertical_motion_with_sticky_column() {
        let mut e = editor("hello world\nab\nlonger line", 5, 10);
        goto(&mut e, 4);
        mv(&mut e, Down);
        assert_eq!(e.cursor(), 10);
        assert_eq!(e.sticky_col(), Some(4));
        mv(&mut e, Down);
        assert_eq!(e.cursor(), 14);
        mv(&mut e, Down);
        assert_eq!(e.cursor(), 19);
        assert_eq!(e.sticky_col(), Some(4));
        mv(&mut e, Up);
        assert_eq!(e.cursor(), 14);
        mv(&mut e, Right);
        assert_eq!(e.sticky_col(), None);
        goto(&mut e, 2);
        mv(&mut e, Up);
        assert_eq!(e.cursor(), 0);
        assert_eq!(e.sticky_col(), None);
        goto(&mut e, 22);
        mv(&mut e, Down);
        assert_eq!(e.cursor(), 26);
        mv(&mut e, Down);
        assert_eq!(e.cursor(), 26);
    }

    #[test]
    fn end_then_down_lands_on_the_shorter_line_end() {
        let text = format!("{}\nshort\nmore", "x".repeat(10));
        let mut e = editor(&text, 10, 10);
        goto(&mut e, 10);
        assert_eq!(e.visual_cursor(), (1, 0));
        mv(&mut e, Down);
        assert_eq!(e.cursor(), 16);
        mv(&mut e, Down);
        assert_eq!(e.cursor(), 21);
    }

    #[test]
    fn page_motions_use_height_minus_one() {
        let text = (0..10)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let mut e = editor(&text, 80, 4);
        mv(&mut e, PageDown);
        assert_eq!(e.cursor(), 6);
        mv(&mut e, PageDown);
        assert_eq!(e.cursor(), 12);
        mv(&mut e, PageUp);
        assert_eq!(e.cursor(), 6);
        mv(&mut e, DocEnd);
        assert_eq!(e.cursor(), text.len());
        mv(&mut e, DocStart);
        assert_eq!(e.cursor(), 0);
    }

    #[test]
    fn scroll_follows_cursor_and_phantom_row_is_reachable() {
        assert_eq!(resolve_scroll(3, 2, 0, None, 5), 0);
        assert_eq!(resolve_scroll(10, 7, 0, None, 5), 3);
        assert_eq!(resolve_scroll(10, 1, 3, None, 5), 1);
        assert_eq!(resolve_scroll(10, 4, 3, None, 5), 3);
        assert_eq!(resolve_scroll(10, 9, 0, Some(99), 5), 5);
        assert_eq!(resolve_scroll(10, 2, 0, Some(1), 5), 1);
        let mut e = editor("a\nb\nc\nd\n", 5, 2);
        mv(&mut e, DocEnd);
        assert_eq!(e.visual_cursor(), (4, 0));
        assert_eq!(e.viewport().scroll, 3);
        let mut e = editor("abcde", 5, 1);
        mv(&mut e, DocEnd);
        assert_eq!(e.visual_cursor(), (1, 0));
        assert_eq!(e.total_rows(), 2);
        assert_eq!(e.viewport().scroll, 1);
    }

    #[test]
    fn edits_reset_derived_state_and_backspace_on_phantom_row() {
        let mut e = editor("a\nb\n", 5, 2);
        mv(&mut e, DocEnd);
        assert_eq!(e.viewport().scroll, 1);
        mv(&mut e, Up);
        assert_eq!(e.sticky_col(), Some(0));
        e.handle(Action::Recenter);
        assert!(e.pinned_scroll().is_some());
        mv(&mut e, DocEnd);
        assert_eq!(e.pinned_scroll(), None);
        e.handle(Action::Delete(DeleteKind::GraphemeBack));
        assert_eq!(e.text(), "a\nb");
        assert_eq!(e.cursor(), 3);
        assert_eq!(e.viewport().scroll, 0);
        assert_eq!(e.layout().width(), 5);
    }

    #[test]
    fn edge_deletes_record_nothing() {
        let mut e = editor("ab", 80, 10);
        e.handle(Action::Delete(DeleteKind::GraphemeBack));
        e.handle(Action::Delete(DeleteKind::WordBack(WordKind::Word)));
        e.handle(Action::Delete(DeleteKind::ToLineStart));
        mv(&mut e, DocEnd);
        e.handle(Action::Delete(DeleteKind::GraphemeFwd));
        e.handle(Action::Delete(DeleteKind::ToLineEnd));
        e.handle(Action::Yank);
        assert_eq!(e.undo_len(), 0);
        assert_eq!(e.text(), "ab");
    }

    #[test]
    fn shift_select_extends_and_collapses() {
        let mut e = editor("one two three", 80, 10);
        e.handle(Action::Select(WordRight));
        assert_eq!(e.selection(), Some(Selection::new(0, 3)));
        e.handle(Action::Select(WordRight));
        assert_eq!(e.selection_range(), Some(0..7));
        e.handle(Action::Select(Left));
        assert_eq!(e.selection_range(), Some(0..6));
        mv(&mut e, Right);
        assert_eq!(e.selection(), None);
        assert_eq!(e.cursor(), 6);
        e.handle(Action::Select(WordLeft));
        assert_eq!(e.selection_range(), Some(4..6));
        mv(&mut e, Left);
        assert_eq!(e.cursor(), 4);
        e.handle(Action::Select(WordRight));
        mv(&mut e, WordRight);
        assert_eq!(e.cursor(), 13);
        e.handle(Action::Select(WordLeft));
        e.handle(Action::Select(WordRight));
        assert_eq!(e.selection(), None);
        goto(&mut e, 5);
        e.handle(Action::Select(LineStart));
        mv(&mut e, LineEnd);
        assert_eq!(e.cursor(), 13);
    }

    #[test]
    fn selection_collapse_keeps_sticky_column_for_vertical_motion() {
        let mut e = editor("abcdef\nab\nabcdef", 80, 10);
        goto(&mut e, 4);
        e.handle(Action::Select(Down));
        assert_eq!(e.selection_range(), Some(4..9));
        assert_eq!(e.sticky_col(), Some(4));
        mv(&mut e, Down);
        assert_eq!(e.selection(), None);
        assert_eq!(e.cursor(), 14);
        e.handle(Action::Select(Up));
        mv(&mut e, Right);
        assert_eq!(e.sticky_col(), None);
    }

    #[test]
    fn type_over_selection_is_one_insert_group() {
        let mut e = editor("hello", 80, 10);
        e.handle(Action::SelectAll);
        assert_eq!(e.selection_range(), Some(0..5));
        assert_eq!(e.cursor(), 5);
        type_str(&mut e, "xy");
        assert_eq!(e.text(), "xy");
        assert_eq!(e.undo_len(), 1);
        e.handle(Action::Undo);
        assert_eq!(e.text(), "hello");
        assert_eq!(e.cursor(), 5);
        e.handle(Action::SelectAll);
        e.handle(Action::Newline);
        assert_eq!(e.text(), "\n");
    }

    #[test]
    fn selection_deletes_kills_yank_copy_cut_and_escape() {
        let mut e = editor("abc def", 80, 10);
        goto(&mut e, 4);
        e.handle(Action::Select(WordRight));
        e.handle(Action::Delete(DeleteKind::GraphemeBack));
        assert_eq!(e.text(), "abc ");
        assert_eq!(e.kill(), None);
        e.handle(Action::Select(WordLeft));
        assert_eq!(e.selection_range(), Some(0..4));
        e.handle(Action::Delete(DeleteKind::ToLineEnd));
        assert_eq!(e.text(), "");
        assert_eq!(e.kill(), Some("abc "));
        type_str(&mut e, "xyz");
        e.handle(Action::Select(Left));
        assert_eq!(e.handle(Action::Copy), Effect::Copy("z".to_string()));
        assert_eq!(e.selection_range(), Some(2..3));
        e.handle(Action::Yank);
        assert_eq!(e.text(), "xyabc ");
        assert_eq!(e.selection(), None);
        e.handle(Action::Select(WordLeft));
        assert_eq!(e.handle(Action::Cut), Effect::Copy("xyabc ".to_string()));
        assert_eq!(e.text(), "");
        assert_eq!(e.kill(), Some("abc "));
        assert_eq!(e.handle(Action::Cut), Effect::Nothing);
        type_str(&mut e, "q");
        e.handle(Action::Select(Left));
        e.handle(Action::Escape);
        assert_eq!(e.selection(), None);
        assert_eq!(e.text(), "q");
    }

    #[test]
    fn yank_with_empty_kill_keeps_selection_and_others_clear_it() {
        let mut e = editor("abc", 80, 10);
        e.handle(Action::Select(Right));
        e.handle(Action::Yank);
        assert_eq!(e.selection_range(), Some(0..1));
        e.handle(Action::Recenter);
        assert_eq!(e.selection(), None);
    }

    struct Lcg(u64);

    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next_u64() % n as u64) as usize
        }
    }

    fn check_invariants(e: &Editor) {
        let text = e.text();
        assert!(is_boundary(text, e.cursor()), "cursor in {text:?}");
        if let Some(sel) = e.selection() {
            assert!(is_boundary(text, sel.anchor));
            assert!(is_boundary(text, sel.head));
            assert_ne!(sel.anchor, sel.head);
        }
        let layout = e.layout();
        assert_eq!(layout.width(), e.viewport().width.max(1));
        for row in layout.rows() {
            assert!(is_boundary(text, row.start));
            assert!(is_boundary(text, row.end));
        }
    }

    const ZWJ: &str = "👩\u{200d}💻";

    #[test]
    fn random_edits_keep_invariants_and_undo_is_exact() {
        let alphabet = ["\n", "\t", " ", "a", "e\u{301}", ZWJ, "한"];
        let motions = [Left, Right, WordLeft, WordRight, Up, Down, DocEnd];
        let mut rng = Lcg(42);
        let mut e = editor("", 6, 3);
        let mut original = e.text().to_string();
        for i in 0..200 {
            match rng.below(12) {
                0..=3 => {
                    let g = alphabet[rng.below(alphabet.len())];
                    let mut chars = g.chars();
                    match (chars.next(), chars.next()) {
                        (Some(c), None) => {
                            e.handle(insert_action(c));
                        }
                        _ => {
                            e.insert_text(g);
                        }
                    }
                }
                4 => {
                    e.handle(Action::Delete(DeleteKind::GraphemeBack));
                }
                5 => {
                    e.handle(Action::Delete(DeleteKind::GraphemeFwd));
                }
                6 => {
                    e.handle(Action::Delete(DeleteKind::WordBack(WordKind::Word)));
                }
                7 => {
                    e.handle(Action::Move(motions[rng.below(motions.len())]));
                }
                8 => {
                    e.handle(Action::Select(motions[rng.below(motions.len())]));
                }
                9 => {
                    e.handle(Action::Undo);
                }
                10 => {
                    e.handle(Action::Redo);
                }
                _ => {
                    e.set_viewport(1 + rng.below(12), 1 + rng.below(6));
                }
            }
            check_invariants(&e);
            if i % 50 == 49 {
                let before = e.text().to_string();
                let mut undone = 0;
                while e.undo_len() > 0 {
                    e.handle(Action::Undo);
                    undone += 1;
                    check_invariants(&e);
                }
                assert_eq!(e.text(), original);
                for _ in 0..undone {
                    e.handle(Action::Redo);
                    check_invariants(&e);
                }
                assert_eq!(e.text(), before);
                let viewport = e.viewport();
                e = editor(&before, viewport.width, viewport.height);
                original = before;
            }
        }
    }
}
