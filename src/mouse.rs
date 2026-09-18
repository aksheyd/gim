use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::buffer::Pos;
use crate::editor::{Editor, Effect, Selection};
use crate::text::{line_range_with_newline, prev_boundary, slice, word_run_at};

pub const CLICK_INTERVAL: Duration = Duration::from_millis(500);
pub const DRAG_SCROLL_INTERVAL: Duration = Duration::from_millis(60);

#[derive(Clone, Copy, Debug)]
struct AutoScroll {
    down: bool,
    col: usize,
    next: Instant,
}

#[derive(Debug, Default)]
pub struct MouseState {
    pub(crate) anchor: Option<Pos>,
    dragging: bool,
    dragged: bool,
    clicks: u8,
    last_click: Option<(Instant, u16, u16)>,
    auto_scroll: Option<AutoScroll>,
}

pub fn wheel_step(height: usize) -> usize {
    if height <= 5 {
        1
    } else if height <= 15 {
        2
    } else {
        3
    }
}

impl Editor {
    pub fn mouse(&mut self, ev: MouseEvent, area: Rect, now: Instant) -> Effect {
        let effect = self.mouse_inner(ev, area, now);
        self.ensure_layout();
        effect
    }

    pub fn next_deadline(&self) -> Option<Duration> {
        self.mouse.auto_scroll.map(|_| DRAG_SCROLL_INTERVAL)
    }

    pub fn advance(&mut self, now: Instant) -> Effect {
        if !self.mouse.dragging {
            self.mouse.auto_scroll = None;
            return Effect::Nothing;
        }
        let effect = self.auto_scroll_step(now);
        self.ensure_layout();
        effect
    }

    pub(crate) fn end_drag(&mut self) {
        self.mouse.dragging = false;
        self.mouse.dragged = false;
        self.mouse.anchor = None;
        self.mouse.auto_scroll = None;
    }

    fn mouse_inner(&mut self, ev: MouseEvent, area: Rect, now: Instant) -> Effect {
        let inside = area.contains(Position::new(ev.column, ev.row));
        match ev.kind {
            MouseEventKind::ScrollUp if inside => self.wheel(false),
            MouseEventKind::ScrollDown if inside => self.wheel(true),
            MouseEventKind::Down(MouseButton::Left) if inside => {
                if self.mouse.dragging {
                    self.drag_to(ev, area, now)
                } else {
                    self.click(ev, area, now)
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.mouse.dragging => {
                self.drag_to(ev, area, now)
            }
            MouseEventKind::Up(MouseButton::Left) if self.mouse.dragging => self.release(),
            _ => Effect::Nothing,
        }
    }

    fn pos_at_cell(&self, area: Rect, column: u16, row: u16) -> Pos {
        let layout = self.layout();
        let text = self.text();
        let vrow = usize::from(row.saturating_sub(area.y)) + self.viewport().scroll;
        if vrow >= layout.row_count() {
            return text.len();
        }
        let col = usize::from(column.saturating_sub(area.x));
        layout.pos_at(text, vrow, col)
    }

    fn click(&mut self, ev: MouseEvent, area: Rect, now: Instant) -> Effect {
        let same_cell = self.mouse.last_click.is_some_and(|(t, c, r)| {
            (c, r) == (ev.column, ev.row) && now.saturating_duration_since(t) <= CLICK_INTERVAL
        });
        self.mouse.clicks = if same_cell {
            self.mouse.clicks % 3 + 1
        } else {
            1
        };
        self.mouse.last_click = Some((now, ev.column, ev.row));
        let pos = self.pos_at_cell(area, ev.column, ev.row);
        self.clear_sticky_and_pin();
        self.set_selection(None);
        self.end_drag();
        match self.mouse.clicks {
            2 => self.double_click(pos),
            3 => self.triple_click(pos),
            _ => {
                self.set_cursor(pos);
                self.mouse.anchor = Some(pos);
                self.mouse.dragging = true;
                Effect::Redraw
            }
        }
    }

    fn double_click(&mut self, pos: Pos) -> Effect {
        let Some(run) = word_run_at(self.text(), pos) else {
            self.set_cursor(pos);
            return Effect::Redraw;
        };
        let copied = slice(self.text(), run.clone()).to_string();
        let last = prev_boundary(self.text(), run.end);
        self.set_selection(Some(Selection::new(run.start, run.end)));
        self.set_cursor(last);
        Effect::Copy(copied)
    }

    fn triple_click(&mut self, pos: Pos) -> Effect {
        let range = line_range_with_newline(self.text(), pos);
        self.set_cursor(pos);
        if range.is_empty() {
            return Effect::Redraw;
        }
        let copied = slice(self.text(), range.clone()).to_string();
        self.set_selection(Some(Selection::new(range.start, range.end)));
        Effect::Copy(copied)
    }

    fn drag_to(&mut self, ev: MouseEvent, area: Rect, now: Instant) -> Effect {
        self.mouse.dragged = true;
        let below = ev.row >= area.bottom();
        let scroll = self.viewport().scroll;
        let from_elsewhere = match self.mouse.anchor {
            Some(anchor) => self.layout().row_of(anchor) != scroll,
            None => false,
        };
        let above = ev.row < area.y || (ev.row == area.y && scroll > 0 && from_elsewhere);
        if above || below {
            let col = usize::from(ev.column.saturating_sub(area.x));
            let next = match self.mouse.auto_scroll {
                Some(a) if a.down == below => a.next,
                _ => now,
            };
            self.mouse.auto_scroll = Some(AutoScroll {
                down: below,
                col,
                next,
            });
            return self.auto_scroll_step(now);
        }
        self.mouse.auto_scroll = None;
        let pos = self.pos_at_cell(area, ev.column, ev.row);
        self.set_head(pos);
        Effect::Redraw
    }

    fn auto_scroll_step(&mut self, now: Instant) -> Effect {
        let Some(auto) = self.mouse.auto_scroll else {
            return Effect::Nothing;
        };
        if now < auto.next {
            return Effect::Nothing;
        }
        let height = self.viewport().height.max(1);
        let max_scroll = self.total_rows().saturating_sub(height);
        let scroll = self.viewport().scroll;
        let target = if auto.down {
            (scroll + 1).min(max_scroll)
        } else {
            scroll.saturating_sub(1)
        };
        let edge_row = if auto.down {
            target + height - 1
        } else {
            target
        };
        let before = self.selection();
        let pos = self.row_pos(edge_row, auto.col);
        self.set_head(pos);
        if target == scroll {
            self.mouse.auto_scroll = None;
            return if self.selection() == before {
                Effect::Nothing
            } else {
                Effect::Redraw
            };
        }
        self.pin_scroll(target);
        self.mouse.auto_scroll = Some(AutoScroll {
            next: now + DRAG_SCROLL_INTERVAL,
            ..auto
        });
        Effect::Redraw
    }

    fn release(&mut self) -> Effect {
        let dragged = self.mouse.dragged;
        self.end_drag();
        if !dragged {
            return Effect::Redraw;
        }
        match self.selection_range() {
            Some(range) => Effect::Copy(slice(self.text(), range).to_string()),
            None => Effect::Redraw,
        }
    }

    fn wheel(&mut self, down: bool) -> Effect {
        let height = self.viewport().height.max(1);
        let step = wheel_step(height);
        let max_scroll = self.total_rows().saturating_sub(height);
        let scroll = self.viewport().scroll;
        let target = if down {
            (scroll + step).min(max_scroll)
        } else {
            scroll.saturating_sub(step)
        };
        if target == scroll {
            return Effect::Nothing;
        }
        self.pin_scroll(target);
        if self.mouse.dragging {
            let edge_row = if down { target + height - 1 } else { target };
            let pos = self.row_pos(edge_row, 0);
            self.set_head(pos);
        }
        Effect::Redraw
    }

    fn row_pos(&self, row: usize, col: usize) -> Pos {
        if row >= self.layout().row_count() {
            self.len()
        } else {
            self.layout().pos_at(self.text(), row, col)
        }
    }

    fn set_head(&mut self, head: Pos) {
        let anchor = self.mouse.anchor.unwrap_or(head);
        let selection = if head == anchor {
            None
        } else {
            Some(Selection::new(anchor, head))
        };
        self.set_selection(selection);
        self.set_cursor(head);
        self.clear_sticky();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crossterm::event::KeyModifiers;

    use crate::keys::{Action, DeleteKind, Motion};

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
        let copied = e.mouse(up(3, 0), area(), t);
        assert_eq!(copied, Effect::Copy("el".to_string()));
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
        let second = click(&mut e, 5, 0, ms(t, 100));
        assert_eq!(second, Effect::Copy("bar".to_string()));
        assert_eq!(e.selection_range(), Some(4..7));
        assert_eq!(e.cursor(), 6);
        let third = click(&mut e, 5, 0, ms(t, 200));
        assert_eq!(third, Effect::Copy("foo bar.\n".to_string()));
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
        let copied = e.mouse(up(0, 1), area(), t);
        assert_eq!(copied, Effect::Copy("a\n".to_string()));
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
        assert_eq!(e.mouse.anchor, Some(0));
    }
}
