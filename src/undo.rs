//! Undo history: inverse deltas grouped into steps.

use crate::buffer::{Buffer, Delta, EditKind, Pos};

/// Maximum number of undo groups kept; the oldest is evicted beyond this.
pub const UNDO_DEPTH: usize = 100;

#[derive(Clone, Debug)]
pub struct UndoGroup {
    pub edits: Vec<Delta>,
    pub cursor_before: Pos,
    pub cursor_after: Pos,
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<UndoGroup>,
    redo: Vec<UndoGroup>,
    last_kind: Option<EditKind>,
    last_cursor_after: Option<Pos>,
    last_insert_ws: Option<bool>,
}

impl History {
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Appends to the open group or starts a new one; clears redo.
    pub fn record(&mut self, delta: Delta) {
        let first_ws = delta.inserted.chars().next().map(char::is_whitespace);
        let last_ws = delta.inserted.chars().next_back().map(char::is_whitespace);
        let new_group = self.undo.is_empty()
            || self.last_kind != Some(delta.kind)
            || self.last_cursor_after != Some(delta.cursor_before)
            || matches!(delta.kind, EditKind::Kill | EditKind::Replace)
            || (delta.kind == EditKind::Insert && first_ws != self.last_insert_ws);
        self.redo.clear();
        if new_group {
            self.undo.push(UndoGroup {
                edits: Vec::new(),
                cursor_before: delta.cursor_before,
                cursor_after: delta.cursor_after,
            });
            if self.undo.len() > UNDO_DEPTH {
                self.undo.remove(0);
            }
        }
        self.last_kind = Some(delta.kind);
        self.last_cursor_after = Some(delta.cursor_after);
        self.last_insert_ws = if delta.kind == EditKind::Insert {
            last_ws
        } else {
            None
        };
        if let Some(group) = self.undo.last_mut() {
            group.cursor_after = delta.cursor_after;
            group.edits.push(delta);
        }
    }

    /// Reverts the newest group; the cursor returns to where the group began.
    pub fn undo(&mut self, buf: &mut Buffer) -> bool {
        let Some(mut group) = self.undo.pop() else {
            return false;
        };
        group.cursor_after = buf.cursor();
        for delta in group.edits.iter().rev() {
            buf.replay(delta, false);
        }
        buf.set_cursor(group.cursor_before);
        self.redo.push(group);
        self.reset_batching();
        true
    }

    /// Re-applies the newest undone group; the cursor returns to where it
    /// was when the group was undone.
    pub fn redo(&mut self, buf: &mut Buffer) -> bool {
        let Some(group) = self.redo.pop() else {
            return false;
        };
        for delta in &group.edits {
            buf.replay(delta, true);
        }
        buf.set_cursor(group.cursor_after);
        self.undo.push(group);
        self.reset_batching();
        true
    }

    /// Forces the next recorded delta to start a new group.
    pub fn reset_batching(&mut self) {
        self.last_kind = None;
        self.last_cursor_after = None;
        self.last_insert_ws = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Edit;

    struct Rig {
        buf: Buffer,
        history: History,
    }

    impl Rig {
        fn new(text: &str) -> Self {
            Rig {
                buf: Buffer::new(text.to_string()),
                history: History::default(),
            }
        }

        fn edit(&mut self, start: Pos, end: Pos, text: &str, kind: EditKind) {
            let edit = Edit {
                range: start..end,
                text: text.to_string(),
                cursor: start + text.len(),
                kind,
            };
            if let Some(delta) = self.buf.apply(edit) {
                self.history.record(delta);
            }
        }

        fn type_str(&mut self, s: &str) {
            for c in s.chars() {
                let p = self.buf.cursor();
                self.edit(p, p, &c.to_string(), EditKind::Insert);
            }
        }
    }

    #[test]
    fn typing_coalesces_by_whitespace_class() {
        let mut r = Rig::new("");
        r.type_str("hello world");
        assert_eq!(r.history.undo_len(), 3);
        r.type_str("\nnext");
        assert_eq!(r.history.undo_len(), 5);
        assert!(r.history.undo(&mut r.buf));
        assert_eq!(r.buf.text(), "hello world\n");
        assert!(r.history.undo(&mut r.buf));
        assert_eq!(r.buf.text(), "hello world");
        assert_eq!(r.buf.cursor(), 11);
    }

    #[test]
    fn kind_change_and_cursor_jump_split_groups() {
        let mut r = Rig::new("");
        r.type_str("abc");
        r.edit(2, 3, "", EditKind::Delete);
        r.edit(1, 2, "", EditKind::Delete);
        assert_eq!(r.history.undo_len(), 2);
        assert_eq!(r.buf.text(), "a");
        r.buf.set_cursor(0);
        r.type_str("x");
        assert_eq!(r.history.undo_len(), 3);
        r.type_str("y");
        assert_eq!(r.history.undo_len(), 3);
        assert_eq!(r.buf.text(), "xya");
    }

    #[test]
    fn kill_and_replace_are_always_discrete() {
        let mut r = Rig::new("one two three");
        r.edit(8, 13, "", EditKind::Kill);
        r.edit(4, 8, "", EditKind::Kill);
        assert_eq!(r.history.undo_len(), 2);
        r.edit(0, 0, "a", EditKind::Replace);
        r.edit(1, 1, "b", EditKind::Replace);
        assert_eq!(r.history.undo_len(), 4);
    }

    #[test]
    fn redo_cleared_on_record_and_cursor_restore_rules() {
        let mut r = Rig::new("");
        r.type_str("ab");
        r.type_str(" ");
        assert_eq!(r.history.undo_len(), 2);
        r.buf.set_cursor(0);
        assert!(r.history.undo(&mut r.buf));
        assert_eq!(r.buf.text(), "ab");
        assert_eq!(r.buf.cursor(), 2);
        assert_eq!(r.history.redo_len(), 1);
        assert!(r.history.redo(&mut r.buf));
        assert_eq!(r.buf.text(), "ab ");
        assert_eq!(r.buf.cursor(), 0);
        assert!(r.history.undo(&mut r.buf));
        r.type_str("z");
        assert_eq!(r.history.redo_len(), 0);
        assert!(!r.history.redo(&mut r.buf));
        assert_eq!(r.history.undo_len(), 2);
    }

    #[test]
    fn depth_is_capped_at_one_hundred() {
        let mut r = Rig::new("");
        for _ in 0..(UNDO_DEPTH + 5) {
            let p = r.buf.cursor();
            r.edit(p, p, "k", EditKind::Kill);
        }
        assert_eq!(r.history.undo_len(), UNDO_DEPTH);
        let mut undone = 0;
        while r.history.undo(&mut r.buf) {
            undone += 1;
        }
        assert_eq!(undone, UNDO_DEPTH);
        assert_eq!(r.buf.text().len(), 5);
    }

    #[test]
    fn empty_history_is_a_no_op() {
        let mut r = Rig::new("x");
        assert!(!r.history.undo(&mut r.buf));
        assert!(!r.history.redo(&mut r.buf));
        assert_eq!(r.buf.text(), "x");
    }
}
