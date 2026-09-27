use gim::buffer::{Buffer, Edit, EditKind};

fn insert(range: std::ops::Range<usize>, text: &str) -> Edit {
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

#[test]
fn fused_cluster_keeps_right_affinity_through_undo_redo() {
    let mut buf = Buffer::new("👩💻".to_string());
    let mid = "👩".len();
    buf.set_cursor(mid);
    let delta = buf.apply(insert(mid..mid, "\u{200d}")).unwrap();
    assert_eq!(buf.text(), "👩\u{200d}💻");
    assert_eq!(buf.cursor(), buf.len());
    buf.replay(&delta, false);
    assert_eq!(buf.text(), "👩💻");
    assert_eq!(buf.cursor(), mid);
    buf.replay(&delta, true);
    assert_eq!(buf.text(), "👩\u{200d}💻");
    assert_eq!(buf.cursor(), buf.len());
}
