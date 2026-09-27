use std::ops::Range;

use crate::text::{ceil_boundary, is_boundary, nearest_boundary, slice};

pub type Pos = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    Insert,
    Delete,
    Kill,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub range: Range<Pos>,
    pub text: String,
    pub cursor: Pos,
    pub kind: EditKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    pub start: Pos,
    pub removed: String,
    pub inserted: String,
    pub cursor_before: Pos,
    pub cursor_after: Pos,
    pub kind: EditKind,
}

#[derive(Clone, Debug, Default)]
pub struct Buffer {
    text: String,
    cursor: Pos,
}

impl Buffer {
    pub fn new(text: String) -> Self {
        Buffer { text, cursor: 0 }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> Pos {
        self.cursor
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn set_cursor(&mut self, pos: Pos) {
        self.cursor = nearest_boundary(&self.text, pos);
    }

    pub fn apply(&mut self, edit: Edit) -> Option<Delta> {
        let len = self.text.len();
        let start = edit.range.start.min(len);
        let end = edit.range.end.clamp(start, len);
        debug_assert!(is_boundary(&self.text, start));
        debug_assert!(is_boundary(&self.text, end));
        let removed = slice(&self.text, start..end);
        if removed.is_empty() && edit.text.is_empty() {
            return None;
        }
        if removed == edit.text && edit.cursor == self.cursor {
            return None;
        }
        let removed = removed.to_string();
        let cursor_before = self.cursor;
        self.text.replace_range(start..end, &edit.text);
        let cursor_after = ceil_boundary(&self.text, edit.cursor);
        self.cursor = cursor_after;
        Some(Delta {
            start,
            removed,
            inserted: edit.text,
            cursor_before,
            cursor_after,
            kind: edit.kind,
        })
    }

    pub fn replay(&mut self, delta: &Delta, forward: bool) {
        let (old, new, cursor) = if forward {
            (&delta.removed, &delta.inserted, delta.cursor_after)
        } else {
            (&delta.inserted, &delta.removed, delta.cursor_before)
        };
        let end = delta.start + old.len();
        debug_assert_eq!(slice(&self.text, delta.start..end), old.as_str());
        self.text.replace_range(delta.start..end, new);
        self.set_cursor(cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(range: Range<Pos>, text: &str) -> Edit {
        Edit {
            cursor: range.start + text.len(),
            range,
            text: text.to_string(),
            kind: EditKind::Insert,
        }
    }

    #[test]
    fn apply_then_replay_round_trips() {
        let mut buf = Buffer::new("hello world".to_string());
        buf.set_cursor(5);
        let delta = buf.apply(insert(5..11, ",")).unwrap();
        assert_eq!(buf.text(), "hello,");
        assert_eq!(buf.cursor(), 6);
        assert_eq!(delta.removed, " world");
        assert_eq!(delta.cursor_before, 5);
        buf.replay(&delta, false);
        assert_eq!(buf.text(), "hello world");
        assert_eq!(buf.cursor(), 5);
        buf.replay(&delta, true);
        assert_eq!(buf.text(), "hello,");
    }

    #[test]
    fn no_ops_return_none() {
        let mut buf = Buffer::new("abc".to_string());
        buf.set_cursor(3);
        assert!(buf.apply(insert(1..1, "")).is_none());
        assert!(buf.apply(insert(0..3, "abc")).is_none());
        buf.set_cursor(0);
        assert!(buf.apply(insert(0..3, "abc")).is_some());
        assert_eq!(buf.cursor(), 3);
    }
}
