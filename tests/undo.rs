use gim::buffer::{Buffer, Edit, EditKind};
use gim::undo::{History, UNDO_DEPTH};

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

    fn edit(&mut self, start: usize, end: usize, text: &str, kind: EditKind) {
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
