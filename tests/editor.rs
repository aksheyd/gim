use gim::editor::{Editor, Effect, Selection, resolve_scroll};
use gim::keys::{Action, DeleteKind, Motion, Motion::*};
use gim::text::{WordKind, is_boundary};

fn editor(text: &str, width: usize, height: usize) -> Editor {
    Editor::new(text.to_string(), width, height)
}

fn insert_action(c: char) -> Action {
    if c == '\n' {
        Action::Newline
    } else {
        Action::Insert(c)
    }
}

fn type_str(e: &mut Editor, s: &str) {
    for c in s.chars() {
        e.handle(insert_action(c));
    }
}

fn mv(e: &mut Editor, m: Motion) {
    e.handle(Action::Move(m));
}

fn goto(e: &mut Editor, pos: usize) {
    e.handle(Action::Move(Motion::DocStart));
    while e.cursor() < pos {
        e.handle(Action::Move(Motion::Right));
    }
}

#[test]
fn line_start_end_chain_with_edge_guards() {
    let mut e = editor("ab\ncd\nef", 80, 10);
    goto(&mut e, 4);
    mv(&mut e, LineStartChain);
    assert_eq!(e.cursor(), 3);
    mv(&mut e, LineStartChain);
    assert_eq!(e.cursor(), 0);
    mv(&mut e, LineStartChain);
    assert_eq!(e.cursor(), 0);
    mv(&mut e, LineEndChain);
    assert_eq!(e.cursor(), 2);
    mv(&mut e, LineEndChain);
    assert_eq!(e.cursor(), 5);
    mv(&mut e, LineEndChain);
    assert_eq!(e.cursor(), 8);
    mv(&mut e, LineEndChain);
    assert_eq!(e.cursor(), 8);
    goto(&mut e, 4);
    mv(&mut e, LineStart);
    assert_eq!(e.cursor(), 3);
    mv(&mut e, LineStart);
    assert_eq!(e.cursor(), 3);
    mv(&mut e, LineEnd);
    assert_eq!(e.cursor(), 5);
    mv(&mut e, LineEnd);
    assert_eq!(e.cursor(), 5);
}

#[test]
fn kill_line_edges_and_yank() {
    let mut e = editor("ab\ncd", 80, 10);
    goto(&mut e, 3);
    e.handle(Action::Delete(DeleteKind::ToLineStart));
    assert_eq!(e.text(), "abcd");
    assert_eq!(e.kill(), Some("\n"));
    e.handle(Action::Delete(DeleteKind::ToLineEnd));
    assert_eq!(e.text(), "ab");
    assert_eq!(e.kill(), Some("cd"));
    assert_eq!(e.undo_len(), 2);
    goto(&mut e, 0);
    e.handle(Action::Yank);
    assert_eq!(e.text(), "cdab");
    assert_eq!(e.cursor(), 2);
    e.handle(Action::Undo);
    assert_eq!(e.text(), "ab");
    assert_eq!(e.kill(), Some("cd"));
    e.handle(Action::Redo);
    assert_eq!(e.text(), "cdab");
    assert_eq!(e.kill(), Some("cd"));
}

#[test]
fn word_deletes_fill_kill_and_grapheme_deletes_do_not() {
    let mut e = editor("foo.bar   ", 80, 10);
    goto(&mut e, 10);
    e.handle(Action::Delete(DeleteKind::WordBack(WordKind::Word)));
    assert_eq!(e.text(), "foo.");
    assert_eq!(e.kill(), Some("bar   "));
    e.handle(Action::Delete(DeleteKind::GraphemeBack));
    assert_eq!(e.text(), "foo");
    assert_eq!(e.kill(), Some("bar   "));
    goto(&mut e, 0);
    e.handle(Action::Delete(DeleteKind::WordFwd(WordKind::BigWord)));
    assert_eq!(e.text(), "");
    assert_eq!(e.kill(), Some("foo"));
}

#[test]
fn vertical_motion_with_sticky_column() {
    let mut e = editor("hello world\nab\nlonger line", 5, 10);
    goto(&mut e, 4);
    mv(&mut e, Down);
    assert_eq!(e.cursor(), 10);
    assert_eq!(e.sticky_col(), Some(4));
    mv(&mut e, Down);
    assert_eq!(e.cursor(), 14);
    mv(&mut e, Down);
    assert_eq!(e.cursor(), 19);
    assert_eq!(e.sticky_col(), Some(4));
    mv(&mut e, Up);
    assert_eq!(e.cursor(), 14);
    mv(&mut e, Right);
    assert_eq!(e.sticky_col(), None);
    goto(&mut e, 2);
    mv(&mut e, Up);
    assert_eq!(e.cursor(), 0);
    assert_eq!(e.sticky_col(), None);
    goto(&mut e, 22);
    mv(&mut e, Down);
    assert_eq!(e.cursor(), 26);
    mv(&mut e, Down);
    assert_eq!(e.cursor(), 26);
}

#[test]
fn end_then_down_lands_on_the_shorter_line_end() {
    let text = format!("{}\nshort\nmore", "x".repeat(10));
    let mut e = editor(&text, 10, 10);
    goto(&mut e, 10);
    assert_eq!(e.visual_cursor(), (1, 0));
    mv(&mut e, Down);
    assert_eq!(e.cursor(), 16);
    mv(&mut e, Down);
    assert_eq!(e.cursor(), 21);
}

#[test]
fn page_motions_use_height_minus_one() {
    let text = (0..10)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let mut e = editor(&text, 80, 4);
    mv(&mut e, PageDown);
    assert_eq!(e.cursor(), 6);
    mv(&mut e, PageDown);
    assert_eq!(e.cursor(), 12);
    mv(&mut e, PageUp);
    assert_eq!(e.cursor(), 6);
    mv(&mut e, DocEnd);
    assert_eq!(e.cursor(), text.len());
    mv(&mut e, DocStart);
    assert_eq!(e.cursor(), 0);
}

#[test]
fn scroll_follows_cursor_and_phantom_row_is_reachable() {
    assert_eq!(resolve_scroll(3, 2, 0, None, 5), 0);
    assert_eq!(resolve_scroll(10, 7, 0, None, 5), 3);
    assert_eq!(resolve_scroll(10, 1, 3, None, 5), 1);
    assert_eq!(resolve_scroll(10, 4, 3, None, 5), 3);
    assert_eq!(resolve_scroll(10, 9, 0, Some(99), 5), 5);
    assert_eq!(resolve_scroll(10, 2, 0, Some(1), 5), 1);
    let mut e = editor("a\nb\nc\nd\n", 5, 2);
    mv(&mut e, DocEnd);
    assert_eq!(e.visual_cursor(), (4, 0));
    assert_eq!(e.viewport().scroll, 3);
    let mut e = editor("abcde", 5, 1);
    mv(&mut e, DocEnd);
    assert_eq!(e.visual_cursor(), (1, 0));
    assert_eq!(e.total_rows(), 2);
    assert_eq!(e.viewport().scroll, 1);
}

#[test]
fn edits_reset_derived_state_and_backspace_on_phantom_row() {
    let mut e = editor("a\nb\n", 5, 2);
    mv(&mut e, DocEnd);
    assert_eq!(e.viewport().scroll, 1);
    mv(&mut e, Up);
    assert_eq!(e.sticky_col(), Some(0));
    e.handle(Action::Recenter);
    assert!(e.pinned_scroll().is_some());
    mv(&mut e, DocEnd);
    assert_eq!(e.pinned_scroll(), None);
    e.handle(Action::Delete(DeleteKind::GraphemeBack));
    assert_eq!(e.text(), "a\nb");
    assert_eq!(e.cursor(), 3);
    assert_eq!(e.viewport().scroll, 0);
    assert_eq!(e.layout().width(), 5);
}

#[test]
fn edge_deletes_record_nothing() {
    let mut e = editor("ab", 80, 10);
    e.handle(Action::Delete(DeleteKind::GraphemeBack));
    e.handle(Action::Delete(DeleteKind::WordBack(WordKind::Word)));
    e.handle(Action::Delete(DeleteKind::ToLineStart));
    mv(&mut e, DocEnd);
    e.handle(Action::Delete(DeleteKind::GraphemeFwd));
    e.handle(Action::Delete(DeleteKind::ToLineEnd));
    e.handle(Action::Yank);
    assert_eq!(e.undo_len(), 0);
    assert_eq!(e.text(), "ab");
}

#[test]
fn shift_select_extends_and_collapses() {
    let mut e = editor("one two three", 80, 10);
    e.handle(Action::Select(WordRight));
    assert_eq!(e.selection(), Some(Selection::new(0, 3)));
    e.handle(Action::Select(WordRight));
    assert_eq!(e.selection_range(), Some(0..7));
    e.handle(Action::Select(Left));
    assert_eq!(e.selection_range(), Some(0..6));
    mv(&mut e, Right);
    assert_eq!(e.selection(), None);
    assert_eq!(e.cursor(), 6);
    e.handle(Action::Select(WordLeft));
    assert_eq!(e.selection_range(), Some(4..6));
    mv(&mut e, Left);
    assert_eq!(e.cursor(), 4);
    e.handle(Action::Select(WordRight));
    mv(&mut e, WordRight);
    assert_eq!(e.cursor(), 13);
    e.handle(Action::Select(WordLeft));
    e.handle(Action::Select(WordRight));
    assert_eq!(e.selection(), None);
    goto(&mut e, 5);
    e.handle(Action::Select(LineStart));
    mv(&mut e, LineEnd);
    assert_eq!(e.cursor(), 13);
}

#[test]
fn selection_collapse_keeps_sticky_column_for_vertical_motion() {
    let mut e = editor("abcdef\nab\nabcdef", 80, 10);
    goto(&mut e, 4);
    e.handle(Action::Select(Down));
    assert_eq!(e.selection_range(), Some(4..9));
    assert_eq!(e.sticky_col(), Some(4));
    mv(&mut e, Down);
    assert_eq!(e.selection(), None);
    assert_eq!(e.cursor(), 14);
    e.handle(Action::Select(Up));
    mv(&mut e, Right);
    assert_eq!(e.sticky_col(), None);
}

#[test]
fn type_over_selection_is_one_insert_group() {
    let mut e = editor("hello", 80, 10);
    e.handle(Action::SelectAll);
    assert_eq!(e.selection_range(), Some(0..5));
    assert_eq!(e.cursor(), 5);
    type_str(&mut e, "xy");
    assert_eq!(e.text(), "xy");
    assert_eq!(e.undo_len(), 1);
    e.handle(Action::Undo);
    assert_eq!(e.text(), "hello");
    assert_eq!(e.cursor(), 5);
    e.handle(Action::SelectAll);
    e.handle(Action::Newline);
    assert_eq!(e.text(), "\n");
}

#[test]
fn selection_deletes_kills_yank_copy_cut_and_escape() {
    let mut e = editor("abc def", 80, 10);
    goto(&mut e, 4);
    e.handle(Action::Select(WordRight));
    e.handle(Action::Delete(DeleteKind::GraphemeBack));
    assert_eq!(e.text(), "abc ");
    assert_eq!(e.kill(), None);
    e.handle(Action::Select(WordLeft));
    assert_eq!(e.selection_range(), Some(0..4));
    e.handle(Action::Delete(DeleteKind::ToLineEnd));
    assert_eq!(e.text(), "");
    assert_eq!(e.kill(), Some("abc "));
    type_str(&mut e, "xyz");
    e.handle(Action::Select(Left));
    assert_eq!(e.handle(Action::Copy), Effect::Copy("z".to_string()));
    assert_eq!(e.selection_range(), Some(2..3));
    e.handle(Action::Yank);
    assert_eq!(e.text(), "xyabc ");
    assert_eq!(e.selection(), None);
    e.handle(Action::Select(WordLeft));
    assert_eq!(e.handle(Action::Cut), Effect::Copy("xyabc ".to_string()));
    assert_eq!(e.text(), "");
    assert_eq!(e.kill(), Some("abc "));
    assert_eq!(e.handle(Action::Cut), Effect::Nothing);
    type_str(&mut e, "q");
    e.handle(Action::Select(Left));
    e.handle(Action::Escape);
    assert_eq!(e.selection(), None);
    assert_eq!(e.text(), "q");
}

#[test]
fn yank_with_empty_kill_keeps_selection_and_others_clear_it() {
    let mut e = editor("abc", 80, 10);
    e.handle(Action::Select(Right));
    e.handle(Action::Yank);
    assert_eq!(e.selection_range(), Some(0..1));
    e.handle(Action::Recenter);
    assert_eq!(e.selection(), None);
}

struct Lcg(u64);

impl Lcg {
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

fn check_invariants(e: &Editor) {
    let text = e.text();
    assert!(is_boundary(text, e.cursor()), "cursor in {text:?}");
    if let Some(sel) = e.selection() {
        assert!(is_boundary(text, sel.anchor));
        assert!(is_boundary(text, sel.head));
        assert_ne!(sel.anchor, sel.head);
    }
    let layout = e.layout();
    assert_eq!(layout.width(), e.viewport().width.max(1));
    for row in layout.rows() {
        assert!(is_boundary(text, row.start));
        assert!(is_boundary(text, row.end));
    }
}

const ZWJ: &str = "👩\u{200d}💻";

#[test]
fn random_edits_keep_invariants_and_undo_is_exact() {
    let alphabet = ["\n", "\t", " ", "a", "e\u{301}", ZWJ, "한"];
    let motions = [Left, Right, WordLeft, WordRight, Up, Down, DocEnd];
    let mut rng = Lcg(42);
    let mut e = editor("", 6, 3);
    let mut original = e.text().to_string();
    for i in 0..200 {
        match rng.below(12) {
            0..=3 => {
                let g = alphabet[rng.below(alphabet.len())];
                let mut chars = g.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => {
                        e.handle(insert_action(c));
                    }
                    _ => {
                        e.insert_text(g);
                    }
                }
            }
            4 => {
                e.handle(Action::Delete(DeleteKind::GraphemeBack));
            }
            5 => {
                e.handle(Action::Delete(DeleteKind::GraphemeFwd));
            }
            6 => {
                e.handle(Action::Delete(DeleteKind::WordBack(WordKind::Word)));
            }
            7 => {
                e.handle(Action::Move(motions[rng.below(motions.len())]));
            }
            8 => {
                e.handle(Action::Select(motions[rng.below(motions.len())]));
            }
            9 => {
                e.handle(Action::Undo);
            }
            10 => {
                e.handle(Action::Redo);
            }
            _ => {
                e.set_viewport(1 + rng.below(12), 1 + rng.below(6));
            }
        }
        check_invariants(&e);
        if i % 50 == 49 {
            let before = e.text().to_string();
            let mut undone = 0;
            while e.undo_len() > 0 {
                e.handle(Action::Undo);
                undone += 1;
                check_invariants(&e);
            }
            assert_eq!(e.text(), original);
            for _ in 0..undone {
                e.handle(Action::Redo);
                check_invariants(&e);
            }
            assert_eq!(e.text(), before);
            let viewport = e.viewport();
            e = editor(&before, viewport.width, viewport.height);
            original = before;
        }
    }
}
