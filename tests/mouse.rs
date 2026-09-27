use gim::editor::{Editor, Effect, Selection};
use gim::keys::{Action, DeleteKind, Motion};
use gim::mouse::{DRAG_SCROLL_INTERVAL, wheel_step};
use gim::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::time::{Duration, Instant};

fn area() -> Rect {
    Rect::new(0, 0, 10, 3)
}

fn editor(text: &str) -> Editor {
    Editor::new(text.to_string(), 10, 3)
}

fn ev(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn down(c: u16, r: u16) -> MouseEvent {
    ev(MouseEventKind::Down(MouseButton::Left), c, r)
}

fn drag(c: u16, r: u16) -> MouseEvent {
    ev(MouseEventKind::Drag(MouseButton::Left), c, r)
}

fn up(c: u16, r: u16) -> MouseEvent {
    ev(MouseEventKind::Up(MouseButton::Left), c, r)
}

fn ms(t: Instant, n: u64) -> Instant {
    t + Duration::from_millis(n)
}

fn click(e: &mut Editor, c: u16, r: u16, t: Instant) -> Effect {
    let effect = e.mouse(down(c, r), area(), t);
    e.mouse(up(c, r), area(), t);
    effect
}

fn scroll(e: &mut Editor, down: bool) -> Effect {
    let kind = if down {
        MouseEventKind::ScrollDown
    } else {
        MouseEventKind::ScrollUp
    };
    e.mouse(ev(kind, 0, 0), area(), Instant::now())
}

#[test]
fn click_places_cursor_and_maps_edges() {
    let mut e = editor("ab 한\ncd");
    let t = Instant::now();
    click(&mut e, 1, 0, t);
    assert_eq!(e.cursor(), 1);
    click(&mut e, 4, 0, ms(t, 1000));
    assert_eq!(e.cursor(), 3);
    click(&mut e, 9, 0, ms(t, 2000));
    assert_eq!(e.cursor(), 6);
    click(&mut e, 3, 2, ms(t, 3000));
    assert_eq!(e.cursor(), 9);
    click(&mut e, 0, 0, ms(t, 5000));
    assert_eq!(e.cursor(), 0);
    let offset = Rect::new(2, 2, 5, 5);
    e.mouse(down(0, 0), offset, ms(t, 6000));
    assert_eq!(e.cursor(), 0);
    assert_eq!(e.mouse(up(0, 0), offset, t), Effect::Nothing);
    let mut empty = editor("");
    click(&mut empty, 5, 1, t);
    assert_eq!(empty.cursor(), 0);
}

#[test]
fn drag_selects_and_release_copies_but_zero_width_is_discarded() {
    let mut e = editor("hello world");
    let t = Instant::now();
    e.mouse(down(1, 0), area(), t);
    e.mouse(drag(4, 0), area(), t);
    assert_eq!(e.selection(), Some(Selection::new(1, 4)));
    assert_eq!(e.cursor(), 4);
    e.mouse(drag(0, 0), area(), t);
    assert_eq!(e.selection(), Some(Selection::new(1, 0)));
    e.mouse(down(3, 0), area(), t);
    assert_eq!(e.selection(), Some(Selection::new(1, 3)));
    assert_eq!(e.mouse(up(3, 0), area(), t), Effect::Redraw);
    assert_eq!(e.selection_range(), Some(1..3));
    e.mouse(down(2, 0), area(), ms(t, 1000));
    e.mouse(drag(2, 0), area(), ms(t, 1000));
    assert_eq!(e.mouse(up(2, 0), area(), t), Effect::Redraw);
    assert_eq!(e.selection(), None);
    e.handle(Action::Delete(DeleteKind::GraphemeBack));
    assert_eq!(e.text(), "hllo world");
    assert_eq!(e.mouse(drag(5, 0), area(), t), Effect::Nothing);
    assert_eq!(e.mouse(up(5, 0), area(), t), Effect::Nothing);
}

#[test]
fn multi_click_selects_word_then_line_and_tracker_resets() {
    let mut e = editor("foo bar.\nnext");
    let t = Instant::now();
    click(&mut e, 5, 0, t);
    assert_eq!(click(&mut e, 5, 0, ms(t, 100)), Effect::Redraw);
    assert_eq!(e.selection_range(), Some(4..7));
    assert_eq!(e.cursor(), 6);
    assert_eq!(click(&mut e, 5, 0, ms(t, 200)), Effect::Redraw);
    assert_eq!(e.selection_range(), Some(0..9));
    assert_eq!(e.cursor(), 5);
    click(&mut e, 5, 0, ms(t, 300));
    assert_eq!(e.selection(), None);
    click(&mut e, 6, 0, ms(t, 350));
    assert_eq!(e.selection(), None);
    click(&mut e, 6, 0, ms(t, 2000));
    assert_eq!(e.selection(), None);
    click(&mut e, 3, 0, ms(t, 3000));
    click(&mut e, 3, 0, ms(t, 3000));
    assert_eq!(e.selection(), None);
    assert_eq!(e.cursor(), 3);
    click(&mut e, 1, 1, ms(t, 4000));
    click(&mut e, 1, 1, ms(t, 4000));
    assert_eq!(e.cursor(), 12);
    click(&mut e, 1, 1, ms(t, 4000));
    assert_eq!(e.selection_range(), Some(9..13));
    assert_eq!(e.cursor(), 10);
}

#[test]
fn wheel_pins_viewport_without_moving_cursor() {
    let mut e = editor("a\nb\nc\nd\ne\nf");
    assert_eq!(e.viewport().scroll, 0);
    assert_eq!(scroll(&mut e, true), Effect::Redraw);
    assert_eq!(e.viewport().scroll, 1);
    assert_eq!(e.pinned_scroll(), Some(1));
    assert_eq!(e.cursor(), 0);
    e.handle(Action::Escape);
    assert_eq!(e.pinned_scroll(), None);
    assert_eq!(e.viewport().scroll, 0);
    scroll(&mut e, true);
    scroll(&mut e, true);
    scroll(&mut e, true);
    assert_eq!(e.viewport().scroll, 3);
    assert_eq!(scroll(&mut e, true), Effect::Nothing);
    let t = Instant::now();
    click(&mut e, 0, 1, t);
    assert_eq!(e.cursor(), 8);
    assert_eq!(e.pinned_scroll(), None);
    assert_eq!(e.viewport().scroll, 3);
    let moved = ev(MouseEventKind::Moved, 0, 0);
    assert_eq!(e.mouse(moved, area(), t), Effect::Nothing);
    assert_eq!(wheel_step(5), 1);
    assert_eq!(wheel_step(15), 2);
    assert_eq!(wheel_step(16), 3);
}

#[test]
fn drag_below_auto_scrolls_and_wheel_during_drag_moves_head() {
    let mut e = editor("a\nb\nc\nd\ne\nf");
    let t = Instant::now();
    e.mouse(down(0, 0), area(), t);
    assert_eq!(e.mouse(drag(0, 3), area(), t), Effect::Redraw);
    assert_eq!(e.viewport().scroll, 1);
    assert_eq!(e.selection_range(), Some(0..6));
    assert_eq!(e.next_deadline(), Some(DRAG_SCROLL_INTERVAL));
    assert_eq!(e.advance(ms(t, 10)), Effect::Nothing);
    assert_eq!(e.advance(t + DRAG_SCROLL_INTERVAL), Effect::Redraw);
    assert_eq!(e.viewport().scroll, 2);
    assert_eq!(e.selection_range(), Some(0..8));
    assert_eq!(e.advance(ms(t, 120)), Effect::Redraw);
    assert_eq!(e.advance(ms(t, 180)), Effect::Nothing);
    assert_eq!(e.next_deadline(), None);
    assert_eq!(e.selection_range(), Some(0..10));
    assert_eq!(e.mouse(drag(0, 0), area(), ms(t, 200)), Effect::Redraw);
    assert_eq!(e.viewport().scroll, 2);
    assert_eq!(e.selection_range(), Some(0..4));
    scroll(&mut e, false);
    assert_eq!(e.viewport().scroll, 1);
    assert_eq!(e.selection_range(), Some(0..2));
    assert_eq!(e.mouse(up(0, 1), area(), t), Effect::Redraw);
    assert_eq!(e.selection_range(), Some(0..2));
    assert_eq!(e.next_deadline(), None);
    assert_eq!(e.advance(t), Effect::Nothing);
}

#[test]
fn drag_along_the_top_row_selects_and_only_a_drag_from_below_scrolls_up() {
    let mut e = editor("a\nhello world\nc\nd\ne\nf");
    let t = Instant::now();
    scroll(&mut e, true);
    assert_eq!(e.viewport().scroll, 1);
    e.mouse(down(1, 0), area(), t);
    e.mouse(drag(4, 0), area(), t);
    assert_eq!(e.viewport().scroll, 1);
    assert_eq!(e.selection_range(), Some(3..6));
    e.mouse(up(4, 0), area(), t);
    e.mouse(down(1, 1), area(), ms(t, 1000));
    assert_eq!(e.mouse(drag(1, 0), area(), ms(t, 1000)), Effect::Redraw);
    assert_eq!(e.viewport().scroll, 0);
    assert_eq!(e.selection_range(), Some(1..9));
}

#[test]
fn resize_mid_drag_keeps_anchor_and_edits_clamp_it() {
    let mut e = editor("hello world again");
    let t = Instant::now();
    e.mouse(down(2, 0), area(), t);
    e.mouse(drag(4, 0), area(), t);
    assert_eq!(e.selection_range(), Some(2..4));
    e.set_viewport(6, 3);
    e.mouse(drag(1, 1), area(), t);
    assert_eq!(e.selection_range(), Some(2..7));
    e.handle(Action::Move(Motion::DocStart));
    e.handle(Action::Delete(DeleteKind::ToLineEnd));
    assert_eq!(e.text(), "");
}
