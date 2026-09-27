use gim::keys::{Action, DeleteKind, Motion, classify};
use gim::{KeyCode, KeyEvent, KeyModifiers};

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const CONTROL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;
const SUPER: KeyModifiers = KeyModifiers::SUPER;
const GRAPHEME_BACK: Action = Action::Delete(DeleteKind::GraphemeBack);
const GRAPHEME_FWD: Action = Action::Delete(DeleteKind::GraphemeFwd);
const WORD_BACK: Action = Action::Delete(DeleteKind::WordBack(gim::text::WordKind::Word));
const WORD_FWD: Action = Action::Delete(DeleteKind::WordFwd(gim::text::WordKind::Word));
const BIG_WORD_BACK: Action = Action::Delete(DeleteKind::WordBack(gim::text::WordKind::BigWord));
const TO_LINE_START: Action = Action::Delete(DeleteKind::ToLineStart);
const TO_LINE_END: Action = Action::Delete(DeleteKind::ToLineEnd);

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
