use gim::wrap::wrap;

fn rows(text: &str, width: usize) -> Vec<std::ops::Range<usize>> {
    wrap(text, width).rows().to_vec()
}

#[test]
fn trailing_spaces_attach_to_the_row_before() {
    assert_eq!(rows("hello world", 5), [0..6, 6..11]);
    assert_eq!(rows("hello   world", 5), [0..8, 8..13]);
    assert_eq!(rows("ab cd ef", 5), [0..6, 6..8]);
}

#[test]
fn hard_breaks_long_words_and_places_wide_graphemes() {
    assert_eq!(rows("abcdefg", 3), [0..3, 3..6, 6..7]);
    assert_eq!(rows("ab cdefgh", 5), [0..3, 3..8, 8..9]);
    assert_eq!(rows("한", 1), vec![0..3]);
    assert_eq!(rows("한한", 1), [0..3, 3..6]);
    assert_eq!(rows("a 한", 2), [0..2, 2..5]);
    assert_eq!(rows(" bc한", 3), [0..1, 1..3, 3..6]);
}

#[test]
fn newlines_and_empty_lines() {
    assert_eq!(rows("", 5), vec![0..0]);
    assert_eq!(rows("a\n\nb", 5), [0..1, 2..2, 3..4]);
    assert_eq!(rows("a\n", 5), [0..1, 2..2]);
    assert_eq!(rows("abc\ndef", usize::MAX), [0..3, 4..7]);
    assert_eq!(rows("a", 0), vec![0..1]);
}

#[test]
fn tabs_are_four_columns_and_zero_width_graphemes_are_free() {
    assert_eq!(rows("a\tb", 6), vec![0..3]);
    assert_eq!(rows("a\tbc", 6), [0..2, 2..4]);
    assert_eq!(rows("a\tb", 5), [0..2, 2..3]);
    let l = wrap("a\u{feff}b", 5);
    assert_eq!(l.col_of("a\u{feff}b", 4), 1);
    assert_eq!(l.pos_at("a\u{feff}b", 0, 1), 4);
}

#[test]
fn row_of_affinity_and_visual_pos_phantom_rule() {
    let text = "hello world";
    let l = wrap(text, 5);
    assert_eq!(l.row_of(5), 0);
    assert_eq!(l.row_of(6), 1);
    assert_eq!(l.visual_pos(text, 4), (0, 4));
    assert_eq!(l.visual_pos(text, 5), (1, 0));
    assert_eq!(l.visual_pos(text, 6), (1, 0));
    assert_eq!(l.visual_pos(text, 11), (2, 0));
    let l = wrap("ab\ncd", 5);
    assert_eq!(l.row_of(2), 0);
    assert_eq!(l.row_of(3), 1);
    assert_eq!(l.visual_pos("ab\ncd", 2), (0, 2));
}

#[test]
fn pos_at_and_visible_end() {
    let text = "hello   world";
    let l = wrap(text, 5);
    assert_eq!(l.visible_end(text, 0), 5);
    assert_eq!(l.visible_end(text, 1), 13);
    assert_eq!(l.pos_at(text, 0, 0), 0);
    assert_eq!(l.pos_at(text, 0, 4), 4);
    assert_eq!(l.pos_at(text, 0, 7), 5);
    assert_eq!(l.pos_at(text, 1, 9), 13);
    assert_eq!(l.pos_at(text, 7, 0), 13);
    let l = wrap("한a", 4);
    assert_eq!(l.pos_at("한a", 0, 1), 0);
    assert_eq!(l.pos_at("한a", 0, 2), 3);
    let text = "ab \n";
    let l = wrap(text, 5);
    assert_eq!(l.visible_end(text, 0), 3);
}
