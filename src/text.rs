use std::borrow::Cow;
use std::ops::Range;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};
use unicode_width::UnicodeWidthStr;

use crate::buffer::Pos;
use crate::wrap::TAB_STOP;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WordKind {
    Word,
    BigWord,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharClass {
    Space,
    Text,
    Symbol,
}

fn char_floor(text: &str, pos: usize) -> usize {
    let mut p = pos.min(text.len());
    while !text.is_char_boundary(p) {
        p -= 1;
    }
    p
}

fn cursor_at(text: &str, pos: usize) -> GraphemeCursor {
    GraphemeCursor::new(pos, text.len(), true)
}

pub fn is_boundary(text: &str, pos: Pos) -> bool {
    if pos > text.len() || !text.is_char_boundary(pos) {
        return false;
    }
    cursor_at(text, pos).is_boundary(text, 0).unwrap_or(false)
}

pub fn prev_boundary(text: &str, pos: Pos) -> Pos {
    let p = char_floor(text, pos);
    if p == 0 {
        return 0;
    }
    match cursor_at(text, p).prev_boundary(text, 0) {
        Ok(Some(b)) => b,
        _ => 0,
    }
}

pub fn next_boundary(text: &str, pos: Pos) -> Pos {
    let p = char_floor(text, pos);
    if p >= text.len() {
        return text.len();
    }
    match cursor_at(text, p).next_boundary(text, 0) {
        Ok(Some(b)) => b,
        _ => text.len(),
    }
}

pub fn floor_boundary(text: &str, pos: Pos) -> Pos {
    let p = char_floor(text, pos);
    if is_boundary(text, p) {
        p
    } else {
        prev_boundary(text, p)
    }
}

pub fn ceil_boundary(text: &str, pos: Pos) -> Pos {
    let p = char_floor(text, pos);
    if p != pos.min(text.len()) || !is_boundary(text, p) {
        next_boundary(text, p)
    } else {
        p
    }
}

pub fn nearest_boundary(text: &str, pos: Pos) -> Pos {
    let pos = pos.min(text.len());
    let lo = floor_boundary(text, pos);
    if lo == pos {
        return lo;
    }
    let hi = next_boundary(text, lo);
    if pos - lo <= hi - pos { lo } else { hi }
}

pub fn slice(text: &str, range: Range<Pos>) -> &str {
    text.get(range).unwrap_or_default()
}

pub fn grapheme_at(text: &str, pos: Pos) -> Option<&str> {
    if pos >= text.len() {
        None
    } else {
        Some(slice(text, pos..next_boundary(text, pos)))
    }
}

pub fn grapheme_before(text: &str, pos: Pos) -> Option<&str> {
    if pos == 0 {
        None
    } else {
        Some(slice(text, prev_boundary(text, pos)..pos))
    }
}

pub fn char_class(grapheme: &str, kind: WordKind) -> CharClass {
    let c = grapheme.chars().next().unwrap_or(' ');
    if c.is_whitespace() {
        CharClass::Space
    } else if kind == WordKind::BigWord || c.is_alphanumeric() || c == '_' {
        CharClass::Text
    } else {
        CharClass::Symbol
    }
}

pub fn word_left(text: &str, pos: Pos, kind: WordKind) -> Pos {
    let mut p = floor_boundary(text, pos);
    while let Some(g) = grapheme_before(text, p) {
        if char_class(g, kind) != CharClass::Space {
            break;
        }
        p -= g.len();
    }
    let Some(first) = grapheme_before(text, p) else {
        return 0;
    };
    let target = char_class(first, kind);
    while let Some(g) = grapheme_before(text, p) {
        if char_class(g, kind) != target {
            break;
        }
        p -= g.len();
    }
    p
}

pub fn word_right(text: &str, pos: Pos, kind: WordKind) -> Pos {
    let mut p = ceil_boundary(text, pos);
    while let Some(g) = grapheme_at(text, p) {
        if char_class(g, kind) != CharClass::Space {
            break;
        }
        p += g.len();
    }
    let Some(first) = grapheme_at(text, p) else {
        return text.len();
    };
    let target = char_class(first, kind);
    while let Some(g) = grapheme_at(text, p) {
        if char_class(g, kind) != target {
            break;
        }
        p += g.len();
    }
    p
}

pub fn line_start(text: &str, pos: Pos) -> Pos {
    let pos = char_floor(text, pos);
    slice(text, 0..pos).rfind('\n').map_or(0, |i| i + 1)
}

pub fn line_end(text: &str, pos: Pos) -> Pos {
    let pos = char_floor(text, pos);
    slice(text, pos..text.len())
        .find('\n')
        .map_or(text.len(), |i| pos + i)
}

pub fn word_run_at(text: &str, pos: Pos) -> Option<Range<Pos>> {
    let pos = floor_boundary(text, pos);
    let g = grapheme_at(text, pos)?;
    let class = char_class(g, WordKind::Word);
    if class == CharClass::Space {
        return None;
    }
    let mut start = pos;
    while let Some(prev) = grapheme_before(text, start) {
        if char_class(prev, WordKind::Word) != class {
            break;
        }
        start -= prev.len();
    }
    let mut end = pos;
    while let Some(next) = grapheme_at(text, end) {
        if char_class(next, WordKind::Word) != class {
            break;
        }
        end += next.len();
    }
    Some(start..end)
}

pub fn line_range_with_newline(text: &str, pos: Pos) -> Range<Pos> {
    let start = line_start(text, pos);
    let end = line_end(text, pos);
    let end = if end < text.len() { end + 1 } else { end };
    start..end
}

pub fn normalize_newlines(s: &str) -> Cow<'_, str> {
    if !s.contains(['\r', '\u{2028}', '\u{2029}']) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{2028}' | '\u{2029}' => out.push('\n'),
            other => out.push(other),
        }
    }
    Cow::Owned(out)
}

pub fn grapheme_width(g: &str) -> usize {
    match g.chars().next() {
        None => 0,
        Some('\t') => TAB_STOP,
        Some(c) if c.is_control() => 1,
        Some(_) => g.width(),
    }
}

pub fn display_width(s: &str) -> usize {
    s.graphemes(true).map(grapheme_width).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(ceil_boundary(&text, inside), 1 + ZWJ.len());
        assert_eq!(nearest_boundary(&text, inside), 1);
        let late = 1 + "👩\u{200d}".len();
        assert_eq!(nearest_boundary(&text, late), 1 + ZWJ.len());
        assert_eq!(nearest_boundary(&text, 2), 1);
        assert_eq!(nearest_boundary(&text, 99), text.len());
        assert_eq!(prev_boundary(&text, 0), 0);
        assert_eq!(next_boundary(&text, text.len()), text.len());
    }
}
