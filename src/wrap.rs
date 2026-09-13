//! Greedy soft wrapping. Rows are byte ranges into the text; a `\n` belongs
//! to no row, and a whitespace run always stays on the row it follows.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::buffer::Pos;
use crate::text::{display_width, grapheme_width};

/// A tab always occupies this many columns.
pub const TAB_STOP: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    width: usize,
    rows: Vec<Range<Pos>>,
}

/// Wraps `text` at `width` columns (clamped to at least 1).
pub fn wrap(text: &str, width: usize) -> Layout {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut line_start = 0;
    loop {
        let rel_end = text[line_start..].find('\n');
        let line_end = rel_end.map_or(text.len(), |i| line_start + i);
        wrap_line(text, line_start, line_end, width, &mut rows);
        if rel_end.is_none() {
            break;
        }
        line_start = line_end + 1;
    }
    Layout { width, rows }
}

fn wrap_line(text: &str, start: Pos, end: Pos, width: usize, rows: &mut Vec<Range<Pos>>) {
    let line = &text[start..end];
    let mut row_start = start;
    let mut col = 0;
    let mut last_break: Option<Pos> = None;
    let mut after_space = false;
    for (i, g) in line.grapheme_indices(true) {
        let pos = start + i;
        let w = grapheme_width(g);
        if g.chars().next().is_some_and(char::is_whitespace) {
            col += w;
            after_space = true;
            continue;
        }
        if after_space {
            last_break = Some(pos);
            after_space = false;
        }
        if col + w > width && pos > row_start {
            let cut = match last_break {
                Some(b) if b > row_start => b,
                _ => pos,
            };
            rows.push(row_start..cut);
            row_start = cut;
            last_break = None;
            col = display_width(&text[cut..pos]);
            // A wide grapheme can still overflow the moved word; hard-break it.
            if col + w > width && pos > row_start {
                rows.push(row_start..pos);
                row_start = pos;
                col = 0;
            }
        }
        col += w;
    }
    rows.push(row_start..end);
}

impl Layout {
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn rows(&self) -> &[Range<Pos>] {
        &self.rows
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Last row whose start is `<= pos`; a cursor on a soft break lands on
    /// the following row, a cursor on a `\n` stays on the row before it.
    pub fn row_of(&self, pos: Pos) -> usize {
        let idx = self.rows.partition_point(|r| r.start <= pos);
        idx.saturating_sub(1)
    }

    /// Display width from the row start to `pos`.
    pub fn col_of(&self, text: &str, pos: Pos) -> usize {
        let row = &self.rows[self.row_of(pos)];
        let pos = pos.clamp(row.start, row.end);
        display_width(&text[row.start..pos])
    }

    /// Screen cell for `pos`, with a column at or past the width shown at the
    /// start of the next row (which may not exist).
    pub fn visual_pos(&self, text: &str, pos: Pos) -> (usize, usize) {
        let row = self.row_of(pos);
        let col = self.col_of(text, pos);
        if col >= self.width {
            (row + 1, 0)
        } else {
            (row, col)
        }
    }

    /// Position of the first grapheme on `row` whose right edge exceeds
    /// `col`; past the painted width this is `visible_end`.
    pub fn pos_at(&self, text: &str, row: usize, col: usize) -> Pos {
        let Some(range) = self.rows.get(row) else {
            return text.len();
        };
        let mut x = 0;
        for (i, g) in text[range.clone()].grapheme_indices(true) {
            if x >= self.width {
                break;
            }
            let w = grapheme_width(g);
            if x + w > col {
                return range.start + i;
            }
            x += w;
        }
        self.visible_end(text, row)
    }

    /// End of the row's visible text: the row end for the last row of a
    /// logical line, else the start of the trailing whitespace run.
    pub fn visible_end(&self, text: &str, row: usize) -> Pos {
        let Some(range) = self.rows.get(row) else {
            return text.len();
        };
        let next = self.rows.get(row + 1);
        let soft = next.is_some_and(|next| next.start == range.end);
        if !soft {
            return range.end;
        }
        let mut end = range.end;
        for (i, g) in text[range.clone()].grapheme_indices(true).rev() {
            if g.chars().next().is_some_and(char::is_whitespace) {
                end = range.start + i;
            } else {
                break;
            }
        }
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str, width: usize) -> Vec<Range<Pos>> {
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
}
