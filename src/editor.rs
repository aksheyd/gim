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
    use crate::keys::Action;

    #[test]
    fn empty_editor_has_no_selection() {
        let e = Editor::new(String::new(), 80, 10);
        assert!(e.is_empty());
        assert!(e.selection().is_none());
    }

    #[test]
    fn insert_and_undo() {
        let mut e = Editor::new(String::new(), 80, 10);
        e.handle(Action::Insert('a'));
        assert_eq!(e.text(), "a");
        e.handle(Action::Undo);
        assert_eq!(e.text(), "");
    }
}
