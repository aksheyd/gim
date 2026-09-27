use std::borrow::Cow;

use gim::text::{
    WordKind, display_width, floor_boundary, grapheme_width, is_boundary, line_end,
    line_range_with_newline, line_start, nearest_boundary, next_boundary, normalize_newlines,
    prev_boundary, slice, word_left, word_right, word_run_at,
};

const ZWJ: &str = "👩\u{200d}💻";
const FLAG: &str = "\u{1f1f0}\u{1f1f7}";
const E_ACUTE: &str = "e\u{301}";

#[test]
fn boundaries_step_over_clusters() {
    let text = format!("a{ZWJ}{FLAG}{E_ACUTE}한");
    let mut p = 0;
    let mut steps = Vec::new();
    while p < text.len() {
        let n = next_boundary(&text, p);
        steps.push(slice(&text, p..n));
        p = n;
    }
    assert_eq!(steps, ["a", ZWJ, FLAG, E_ACUTE, "한"]);
    let mut back = Vec::new();
    while p > 0 {
        let b = prev_boundary(&text, p);
        back.push(slice(&text, b..p));
        p = b;
    }
    back.reverse();
    assert_eq!(back, steps);
}

#[test]
fn floor_ceil_nearest_inside_a_cluster() {
    let text = format!("x{ZWJ}y");
    let inside = 1 + "👩".len();
    assert!(!is_boundary(&text, inside));
    assert_eq!(floor_boundary(&text, inside), 1);
    assert_eq!(gim::text::ceil_boundary(&text, inside), 1 + ZWJ.len());
    assert_eq!(nearest_boundary(&text, inside), 1);
    let late = 1 + "👩\u{200d}".len();
    assert_eq!(nearest_boundary(&text, late), 1 + ZWJ.len());
    assert_eq!(nearest_boundary(&text, 2), 1);
    assert_eq!(nearest_boundary(&text, 99), text.len());
    assert_eq!(prev_boundary(&text, 0), 0);
    assert_eq!(next_boundary(&text, text.len()), text.len());
}

#[test]
fn word_kinds() {
    let text = "foo.bar   ";
    assert_eq!(word_left(text, 10, WordKind::Word), 4);
    assert_eq!(word_left(text, 4, WordKind::Word), 3);
    assert_eq!(word_left(text, 3, WordKind::Word), 0);
    assert_eq!(word_left(text, 10, WordKind::BigWord), 0);
    assert_eq!(word_right("   foo.bar", 0, WordKind::Word), 6);
    assert_eq!(word_right("   foo.bar", 6, WordKind::Word), 7);
    assert_eq!(word_right("   foo.bar", 0, WordKind::BigWord), 10);
    assert_eq!(word_left("   ", 3, WordKind::Word), 0);
    assert_eq!(word_right("abc", 3, WordKind::Word), 3);
}

#[test]
fn words_cross_lines_and_scripts() {
    let text = "foo\n\nbar_1 é한";
    assert_eq!(word_left(text, 5, WordKind::Word), 0);
    assert_eq!(word_right(text, 3, WordKind::Word), 10);
    assert_eq!(word_right(text, 10, WordKind::Word), text.len());
    assert_eq!(word_left(text, text.len(), WordKind::Word), 11);
}

#[test]
fn lines() {
    let text = "ab\ncd\n";
    assert_eq!(line_start(text, 4), 3);
    assert_eq!(line_end(text, 4), 5);
    assert_eq!(line_start(text, 0), 0);
    assert_eq!(line_end(text, 6), 6);
    assert_eq!(line_range_with_newline(text, 4), 3..6);
    assert_eq!(line_range_with_newline(text, 6), 6..6);
    assert_eq!(line_range_with_newline("x", 0), 0..1);
}

#[test]
fn word_runs_for_double_click() {
    let text = "hi there.. x";
    assert_eq!(word_run_at(text, 0), Some(0..2));
    assert_eq!(word_run_at(text, 2), None);
    assert_eq!(word_run_at(text, 4), Some(3..8));
    assert_eq!(word_run_at(text, 9), Some(8..10));
    assert_eq!(word_run_at(text, text.len()), None);
}

#[test]
fn newline_normalisation() {
    assert!(matches!(normalize_newlines("a\nb"), Cow::Borrowed(_)));
    assert_eq!(normalize_newlines("a\rb\rc"), "a\nb\nc");
    assert_eq!(normalize_newlines("a\r\nb\r\n"), "a\nb\n");
    assert_eq!(normalize_newlines("a\u{2028}b\u{2029}"), "a\nb\n");
    assert_eq!(normalize_newlines("\ta\r\n"), "\ta\n");
}

#[test]
fn widths() {
    assert_eq!(grapheme_width("\t"), gim::wrap::TAB_STOP);
    assert_eq!(grapheme_width("\u{7}"), 1);
    assert_eq!(grapheme_width("\u{85}"), 1);
    assert_eq!(grapheme_width("한"), 2);
    assert_eq!(grapheme_width(ZWJ), 2);
    assert_eq!(grapheme_width(E_ACUTE), 1);
    assert_eq!(grapheme_width("\u{301}"), 0);
    assert_eq!(grapheme_width("\u{feff}"), 0);
    assert_eq!(display_width("a\tb한"), 8);
}
