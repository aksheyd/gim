use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::text::WordKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    LineStartChain,
    LineEndChain,
    Up,
    Down,
    PageUp,
    PageDown,
    DocStart,
    DocEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteKind {
    GraphemeBack,
    GraphemeFwd,
    WordBack(WordKind),
    WordFwd(WordKind),
    ToLineStart,
    ToLineEnd,
}

impl DeleteKind {
    pub fn is_kill(self) -> bool {
        !matches!(self, DeleteKind::GraphemeBack | DeleteKind::GraphemeFwd)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Insert(char),
    Newline,
    Move(Motion),
    Select(Motion),
    Delete(DeleteKind),
    Yank,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Save,
    Quit,
    Recenter,
    Escape,
}

const GRAPHEME_BACK: Action = Action::Delete(DeleteKind::GraphemeBack);
const GRAPHEME_FWD: Action = Action::Delete(DeleteKind::GraphemeFwd);
const WORD_BACK: Action = Action::Delete(DeleteKind::WordBack(WordKind::Word));
const WORD_FWD: Action = Action::Delete(DeleteKind::WordFwd(WordKind::Word));
const BIG_WORD_BACK: Action = Action::Delete(DeleteKind::WordBack(WordKind::BigWord));
const TO_LINE_START: Action = Action::Delete(DeleteKind::ToLineStart);
const TO_LINE_END: Action = Action::Delete(DeleteKind::ToLineEnd);

const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const CONTROL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;
const SUPER: KeyModifiers = KeyModifiers::SUPER;

fn chord(raw: KeyModifiers) -> bool {
    raw.intersects(CONTROL | ALT | SUPER)
}

fn only(raw: KeyModifiers, want: KeyModifiers) -> bool {
    raw.difference(SHIFT) == want
}

pub fn classify(key: KeyEvent) -> Option<Action> {
    let raw = key.modifiers.intersection(SHIFT | CONTROL | ALT | SUPER);
    if let KeyCode::Char(c) = key.code
        && !chord(raw)
        && !c.is_control()
    {
        let c = if raw.contains(SHIFT) && c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c
        };
        return Some(Action::Insert(c));
    }
    let (code, raw) = match key.code {
        KeyCode::Char(c) if c.is_ascii_uppercase() => {
            (KeyCode::Char(c.to_ascii_lowercase()), raw | SHIFT)
        }
        other => (other, raw),
    };
    let shift = raw.contains(SHIFT);
    if let Some(motion) = motion_of(code, raw) {
        return Some(if shift {
            Action::Select(motion)
        } else {
            Action::Move(motion)
        });
    }
    let ctrl = raw.contains(CONTROL);
    let alt = raw.contains(ALT);
    let sup = raw.contains(SUPER);
    let action = match code {
        KeyCode::Enter => Action::Newline,
        KeyCode::Char('j' | 'm') if only(raw, CONTROL) => Action::Newline,
        KeyCode::Tab if !chord(raw) => Action::Insert('\t'),
        KeyCode::Backspace if matches!(raw, ALT | CONTROL) => WORD_BACK,
        KeyCode::Backspace if raw == SUPER => TO_LINE_START,
        KeyCode::Backspace => GRAPHEME_BACK,
        KeyCode::Char('h') if ctrl && alt => WORD_BACK,
        KeyCode::Char('h') if ctrl => GRAPHEME_BACK,
        KeyCode::Char('\u{7f}' | '\u{8}') => GRAPHEME_BACK,
        KeyCode::Delete if chord(raw) => WORD_FWD,
        KeyCode::Delete => GRAPHEME_FWD,
        KeyCode::Char('d') if only(raw, CONTROL) => GRAPHEME_FWD,
        KeyCode::Char('d') if only(raw, ALT) || sup => WORD_FWD,
        KeyCode::Char('w') if only(raw, CONTROL) => BIG_WORD_BACK,
        KeyCode::Char('u') if only(raw, CONTROL) => TO_LINE_START,
        KeyCode::Char('k') if only(raw, CONTROL) => TO_LINE_END,
        KeyCode::Char('y') if only(raw, CONTROL) => Action::Yank,
        KeyCode::Char('z') if (ctrl || sup) && !shift => Action::Undo,
        KeyCode::Char('z') if ctrl || sup || only(raw, ALT) => Action::Redo,
        KeyCode::Char('r') if only(raw, CONTROL) => Action::Redo,
        KeyCode::Char('c') if ctrl || sup => Action::Copy,
        KeyCode::Char('x') if ctrl || sup => Action::Cut,
        KeyCode::Char('v') if ctrl || sup => Action::Paste,
        KeyCode::Char('a') if sup || only(raw, ALT) => Action::SelectAll,
        KeyCode::Char('s') if ctrl || sup => Action::Save,
        KeyCode::Char('q') if only(raw, CONTROL) => Action::Quit,
        KeyCode::Char('l') if only(raw, CONTROL) => Action::Recenter,
        KeyCode::Esc => Action::Escape,
        KeyCode::Char('[') if only(raw, CONTROL) => Action::Escape,
        _ => return None,
    };
    Some(action)
}

fn motion_of(code: KeyCode, raw: KeyModifiers) -> Option<Motion> {
    let ctrl = raw.contains(CONTROL);
    let sup = raw.contains(SUPER);
    let word = raw.contains(ALT) || ctrl;
    let motion = match code {
        KeyCode::Left if word => Motion::WordLeft,
        KeyCode::Left => Motion::Left,
        KeyCode::Right if word => Motion::WordRight,
        KeyCode::Right => Motion::Right,
        KeyCode::Up if sup => Motion::DocStart,
        KeyCode::Up => Motion::Up,
        KeyCode::Down if sup => Motion::DocEnd,
        KeyCode::Down => Motion::Down,
        KeyCode::Home if ctrl => Motion::DocStart,
        KeyCode::Home => Motion::LineStart,
        KeyCode::End if ctrl => Motion::DocEnd,
        KeyCode::End => Motion::LineEnd,
        KeyCode::PageUp => Motion::PageUp,
        KeyCode::PageDown => Motion::PageDown,
        KeyCode::Char('b') if only(raw, CONTROL) => Motion::Left,
        KeyCode::Char('b') if only(raw, ALT) => Motion::WordLeft,
        KeyCode::Char('f') if only(raw, CONTROL) => Motion::Right,
        KeyCode::Char('f') if only(raw, ALT) => Motion::WordRight,
        KeyCode::Char('a') if only(raw, CONTROL) => Motion::LineStartChain,
        KeyCode::Char('e') if only(raw, CONTROL) => Motion::LineEndChain,
        KeyCode::Char('p') if only(raw, CONTROL) => Motion::Up,
        KeyCode::Char('n') if only(raw, CONTROL) => Motion::Down,
        KeyCode::Char('\u{2}') if !chord(raw) => Motion::Left,
        KeyCode::Char('\u{6}') if !chord(raw) => Motion::Right,
        _ => return None,
    };
    Some(motion)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyModifiers = KeyModifiers::NONE;

    fn k(code: KeyCode, mods: KeyModifiers) -> Option<Action> {
        classify(KeyEvent::new(code, mods))
    }

    fn ch(c: char, mods: KeyModifiers) -> Option<Action> {
        k(KeyCode::Char(c), mods)
    }

    fn mv(m: Motion) -> Option<Action> {
        Some(Action::Move(m))
    }

    fn sel(m: Motion) -> Option<Action> {
        Some(Action::Select(m))
    }

    #[test]
    fn text_keys() {
        assert_eq!(ch('a', NONE), Some(Action::Insert('a')));
        assert_eq!(ch('a', SHIFT), Some(Action::Insert('A')));
        assert_eq!(ch('A', SHIFT), Some(Action::Insert('A')));
        assert_eq!(ch('∂', NONE), Some(Action::Insert('∂')));
        assert_eq!(k(KeyCode::Tab, NONE), Some(Action::Insert('\t')));
        assert_eq!(k(KeyCode::BackTab, SHIFT), None);
        assert_eq!(k(KeyCode::Enter, NONE), Some(Action::Newline));
        assert_eq!(k(KeyCode::Enter, CONTROL | SHIFT), Some(Action::Newline));
        assert_eq!(ch('j', CONTROL), Some(Action::Newline));
        assert_eq!(ch('m', CONTROL), Some(Action::Newline));
    }

    #[test]
    fn deletes() {
        assert_eq!(k(KeyCode::Backspace, NONE), Some(GRAPHEME_BACK));
        assert_eq!(k(KeyCode::Backspace, SHIFT), Some(GRAPHEME_BACK));
        assert_eq!(k(KeyCode::Backspace, ALT), Some(WORD_BACK));
        assert_eq!(k(KeyCode::Backspace, CONTROL), Some(WORD_BACK));
        assert_eq!(k(KeyCode::Backspace, SUPER), Some(TO_LINE_START));
        assert_eq!(ch('h', CONTROL), Some(GRAPHEME_BACK));
        assert_eq!(ch('h', CONTROL | ALT), Some(WORD_BACK));
        assert_eq!(ch('\u{8}', NONE), Some(GRAPHEME_BACK));
        assert_eq!(k(KeyCode::Delete, NONE), Some(GRAPHEME_FWD));
        assert_eq!(k(KeyCode::Delete, ALT), Some(WORD_FWD));
        assert_eq!(ch('d', CONTROL), Some(GRAPHEME_FWD));
        assert_eq!(ch('d', ALT), Some(WORD_FWD));
        assert_eq!(ch('w', CONTROL), Some(BIG_WORD_BACK));
        assert_eq!(ch('u', CONTROL), Some(TO_LINE_START));
        assert_eq!(ch('k', CONTROL), Some(TO_LINE_END));
        assert_eq!(ch('y', CONTROL), Some(Action::Yank));
    }

    #[test]
    fn motions_and_selection() {
        assert_eq!(k(KeyCode::Left, NONE), mv(Motion::Left));
        assert_eq!(k(KeyCode::Left, SHIFT), sel(Motion::Left));
        assert_eq!(k(KeyCode::Left, ALT), mv(Motion::WordLeft));
        assert_eq!(k(KeyCode::Right, CONTROL | SHIFT), sel(Motion::WordRight));
        assert_eq!(ch('b', CONTROL), mv(Motion::Left));
        assert_eq!(ch('\u{2}', NONE), mv(Motion::Left));
        assert_eq!(ch('\u{6}', NONE), mv(Motion::Right));
        assert_eq!(ch('f', ALT), mv(Motion::WordRight));
        assert_eq!(ch('B', ALT), sel(Motion::WordLeft));
        assert_eq!(ch('a', CONTROL), mv(Motion::LineStartChain));
        assert_eq!(ch('e', CONTROL | SHIFT), sel(Motion::LineEndChain));
        assert_eq!(k(KeyCode::Home, NONE), mv(Motion::LineStart));
        assert_eq!(k(KeyCode::End, SHIFT), sel(Motion::LineEnd));
        assert_eq!(k(KeyCode::Home, CONTROL), mv(Motion::DocStart));
        assert_eq!(k(KeyCode::Up, SUPER), mv(Motion::DocStart));
        assert_eq!(k(KeyCode::Down, CONTROL), mv(Motion::Down));
        assert_eq!(ch('p', CONTROL), mv(Motion::Up));
        assert_eq!(ch('n', CONTROL), mv(Motion::Down));
        assert_eq!(k(KeyCode::PageUp, NONE), mv(Motion::PageUp));
    }

    #[test]
    fn app_chords() {
        assert_eq!(ch('z', CONTROL), Some(Action::Undo));
        assert_eq!(ch('z', SUPER), Some(Action::Undo));
        assert_eq!(ch('Z', CONTROL), Some(Action::Redo));
        assert_eq!(ch('z', CONTROL | SHIFT), Some(Action::Redo));
        assert_eq!(ch('z', ALT), Some(Action::Redo));
        assert_eq!(ch('r', CONTROL), Some(Action::Redo));
        assert_eq!(ch('c', CONTROL), Some(Action::Copy));
        assert_eq!(ch('x', SUPER), Some(Action::Cut));
        assert_eq!(ch('v', CONTROL), Some(Action::Paste));
        assert_eq!(ch('v', CONTROL | SHIFT), Some(Action::Paste));
        assert_eq!(ch('a', SUPER), Some(Action::SelectAll));
        assert_eq!(ch('a', ALT), Some(Action::SelectAll));
        assert_eq!(ch('s', CONTROL), Some(Action::Save));
        assert_eq!(ch('q', CONTROL), Some(Action::Quit));
        assert_eq!(ch('l', CONTROL), Some(Action::Recenter));
        assert_eq!(k(KeyCode::Esc, NONE), Some(Action::Escape));
        assert_eq!(ch('[', CONTROL), Some(Action::Escape));
        assert_eq!(ch('g', CONTROL), None);
        assert_eq!(ch('/', CONTROL), None);
        assert_eq!(ch('f', SUPER), None);
        assert_eq!(k(KeyCode::F(1), NONE), None);
    }
}
