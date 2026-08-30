//! Shared plumbing for scrollable text viewers (diff, blame):
//! vertical/horizontal scroll state, incremental search, footer layout.

use super::EventState;
use super::text_search::{InputOutcome, TextSearch};
use crate::keys::{KeyAction, key_match};
use crate::ui::{self, style::Theme};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use std::cell::Cell;

/// Outcome of feeding one event to [`TextView::search_event`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchOutcome {
    /// Not search business; the caller should handle the event itself.
    Ignored,
    /// Search business, but no jump (typing without a match, `/`, Enter).
    Consumed,
    /// The search jumped to a match on this line; the caller reveals it
    /// (scroll offset for the diff view, cursor for the blame popup).
    Reveal(usize),
}

/// Shared base for scrollable text viewers: scroll offsets, the widest-line
/// width used to clamp the horizontal offset, and incremental-search state.
#[derive(Default)]
pub struct TextView {
    pub scroll: Cell<usize>,
    /// Horizontal scroll offset in display columns (`h`/`l`)
    pub hscroll: Cell<usize>,
    /// Display width of the widest rendered line (for clamping `hscroll`)
    pub max_width: Cell<usize>,
    /// Incremental search over the text content (`/`, n/N)
    pub search: TextSearch,
}

impl TextView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Zero both scroll offsets and the width cache; drop the search.
    pub fn reset(&mut self) {
        self.scroll.set(0);
        self.hscroll.set(0);
        self.max_width.set(0);
        self.search.clear();
    }

    /// Scrolling keys: `j`/`k`/↓/↑ step 1 line, `J`/`K`/PgDn/PgUp step 20,
    /// `g`/Home top, `G`/End bottom, `h`/`l`/←/→ scroll ±8 columns.
    /// Consumes those keys; everything else falls through.
    pub fn scroll_event(&self, ev: &Event, len: usize) -> EventState {
        let Event::Key(k) = ev else {
            return EventState::not_consumed();
        };
        let mut scroll = self.scroll.get();
        if key_match(k, KeyAction::MoveDown) || key_match(k, KeyAction::PageDown) {
            scroll = scroll.saturating_add(if key_match(k, KeyAction::PageDown) {
                20
            } else {
                1
            });
        } else if key_match(k, KeyAction::MoveUp) || key_match(k, KeyAction::PageUp) {
            scroll = scroll.saturating_sub(if key_match(k, KeyAction::PageUp) {
                20
            } else {
                1
            });
        } else if key_match(k, KeyAction::Home) {
            scroll = 0;
        } else if key_match(k, KeyAction::End) {
            scroll = len;
        } else if key_match(k, KeyAction::MoveLeft) {
            self.hscroll.set(self.hscroll.get().saturating_sub(8));
            return EventState::consumed();
        } else if key_match(k, KeyAction::MoveRight) {
            // right bound is applied at draw time (needs the area width)
            self.hscroll.set(self.hscroll.get().saturating_add(8));
            return EventState::consumed();
        } else {
            return EventState::not_consumed();
        }
        self.scroll.set(scroll.min(len));
        EventState::consumed()
    }

    /// Search input (`/`, live typing, `n`/`N`) against `lines`.
    /// `from_line` is where the incremental search starts looking (the
    /// diff's scroll offset, the blame cursor). Esc is *not* handled here —
    /// it interacts with closing the popup, which the caller owns.
    pub fn search_event(&mut self, ev: &Event, lines: &[&str], from_line: usize) -> SearchOutcome {
        if self.search.is_input_mode() {
            if self.search.input_event(ev) == InputOutcome::Changed {
                self.search.recompute(lines, from_line);
                if let Some(line) = self.search.current_match_line() {
                    return SearchOutcome::Reveal(line);
                }
            }
            return SearchOutcome::Consumed;
        }
        let Event::Key(k) = ev else {
            return SearchOutcome::Ignored;
        };
        if key_match(k, KeyAction::Filter) {
            self.search.start_input();
            return SearchOutcome::Consumed;
        }
        // plain char, no ctrl/alt: macOS Option+n reports Char('n')+ALT
        // and must not trigger search cycling (same hygiene as is_key)
        let plain = |c: char| {
            k.code == KeyCode::Char(c)
                && !k
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        };
        let jump = if plain('n') {
            self.search.next_match()
        } else if plain('N') {
            self.search.prev_match()
        } else {
            return SearchOutcome::Ignored;
        };
        match jump {
            Some(line) => SearchOutcome::Reveal(line),
            None => SearchOutcome::Consumed,
        }
    }

    /// Carve off the search footer (while a search is active) and clamp
    /// both scroll offsets against the content, writing them back.
    /// Returns the content area, the footer area and the clamped scroll.
    pub fn layout(&self, inner: Rect, total: usize) -> (Rect, Option<Rect>, usize) {
        let (content, footer) = ui::split_search_footer(inner, self.search.is_active());
        let scroll = ui::clamp_scroll(self.scroll.get(), total, content.height as usize);
        self.scroll.set(scroll);
        self.clamp_hscroll(content);
        (content, footer, scroll)
    }

    /// [`TextView::layout`] variant for cursor-driven views (blame): the
    /// scroll offset follows `cursor` so it stays inside the visible window.
    pub fn layout_with_cursor(
        &self,
        inner: Rect,
        total: usize,
        cursor: usize,
    ) -> (Rect, Option<Rect>, usize) {
        let (content, footer) = ui::split_search_footer(inner, self.search.is_active());
        let scroll = ui::scroll_follow(cursor, self.scroll.get(), total, content.height as usize);
        self.scroll.set(scroll);
        self.clamp_hscroll(content);
        (content, footer, scroll)
    }

    /// Clamp the horizontal offset against the widest line (needs the area
    /// width, so it can only happen at draw time) and write it back.
    fn clamp_hscroll(&self, content: Rect) {
        let h_off = ui::clamp_hscroll(
            self.hscroll.get(),
            self.max_width.get(),
            content.width as usize,
        );
        self.hscroll.set(h_off);
    }

    /// Draw the `/pattern  [x/y]` footer line.
    pub fn draw_search_footer(&self, f: &mut Frame, footer: Rect, theme: &Theme) {
        let line = Line::from(Span::styled(self.search.status_text(), theme.info));
        f.buffer_mut()
            .set_line(footer.x, footer.y, &line, footer.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;

    const LINES: [&str; 4] = ["foo bar", "nothing here", "bar foo bar", "last foo"];

    #[test]
    fn scroll_event_steps_and_clamps() {
        let tv = TextView::new();
        let len = 100;
        // j/↓ step 1, k/↑ step back
        assert!(tv.scroll_event(&ts::key(KeyCode::Char('j')), len).consumed);
        assert_eq!(tv.scroll.get(), 1);
        assert!(tv.scroll_event(&ts::key(KeyCode::Down), len).consumed);
        assert_eq!(tv.scroll.get(), 2);
        assert!(tv.scroll_event(&ts::key(KeyCode::Char('k')), len).consumed);
        assert!(tv.scroll_event(&ts::key(KeyCode::Up), len).consumed);
        assert_eq!(tv.scroll.get(), 0);
        // k saturates at 0
        tv.scroll_event(&ts::key(KeyCode::Char('k')), len);
        assert_eq!(tv.scroll.get(), 0);
        // J/K/PgDn/PgUp step 20
        tv.scroll_event(&ts::key(KeyCode::Char('J')), len);
        assert_eq!(tv.scroll.get(), 20);
        tv.scroll_event(&ts::key(KeyCode::PageDown), len);
        assert_eq!(tv.scroll.get(), 40);
        tv.scroll_event(&ts::key(KeyCode::Char('K')), len);
        assert_eq!(tv.scroll.get(), 20);
        tv.scroll_event(&ts::key(KeyCode::PageUp), len);
        assert_eq!(tv.scroll.get(), 0);
        // g/Home top, G/End bottom (bounded by len)
        tv.scroll_event(&ts::key(KeyCode::Char('G')), len);
        assert_eq!(tv.scroll.get(), len);
        tv.scroll_event(&ts::key(KeyCode::Char('g')), len);
        assert_eq!(tv.scroll.get(), 0);
        tv.scroll_event(&ts::key(KeyCode::End), len);
        assert_eq!(tv.scroll.get(), len);
        tv.scroll_event(&ts::key(KeyCode::Home), len);
        assert_eq!(tv.scroll.get(), 0);
        // h/l/←/→ scroll horizontally by 8 columns, saturating at 0
        tv.scroll_event(&ts::key(KeyCode::Char('h')), len);
        assert_eq!(tv.hscroll.get(), 0);
        tv.scroll_event(&ts::key(KeyCode::Char('l')), len);
        assert_eq!(tv.hscroll.get(), 8);
        tv.scroll_event(&ts::key(KeyCode::Right), len);
        assert_eq!(tv.hscroll.get(), 16);
        tv.scroll_event(&ts::key(KeyCode::Left), len);
        assert_eq!(tv.hscroll.get(), 8);
        // anything else is not consumed
        assert!(!tv.scroll_event(&ts::key(KeyCode::Char('x')), len).consumed);
        assert!(!tv.scroll_event(&Event::Paste("p".into()), len).consumed);
    }

    #[test]
    fn search_event_outcomes() {
        let mut tv = TextView::new();
        let lines: Vec<&str> = LINES.to_vec();
        // unrelated key: ignored
        assert_eq!(
            tv.search_event(&ts::key(KeyCode::Char('x')), &lines, 0),
            SearchOutcome::Ignored
        );
        // non-key events outside input mode are ignored too
        assert_eq!(
            tv.search_event(&Event::Paste("foo".into()), &lines, 0),
            SearchOutcome::Ignored
        );
        // '/' starts input mode
        assert_eq!(
            tv.search_event(&ts::key(KeyCode::Char('/')), &lines, 0),
            SearchOutcome::Consumed
        );
        assert!(tv.search.is_input_mode());
        // typing a pattern with matches reveals the first one at/after from_line
        assert_eq!(
            tv.search_event(&ts::key(KeyCode::Char('f')), &lines, 0),
            SearchOutcome::Reveal(0)
        );
        tv.search_event(&ts::key(KeyCode::Char('o')), &lines, 0);
        assert_eq!(
            tv.search_event(&ts::key(KeyCode::Char('o')), &lines, 1),
            SearchOutcome::Reveal(2)
        );
        // a pattern without matches consumes but does not reveal
        tv.search.start_input();
        for c in "zzz".chars() {
            assert_eq!(
                tv.search_event(&ts::key(KeyCode::Char(c)), &lines, 0),
                SearchOutcome::Consumed
            );
        }
        // Enter confirms: consumed, no reveal
        assert_eq!(
            tv.search_event(&ts::key(KeyCode::Enter), &lines, 0),
            SearchOutcome::Consumed
        );
        // back to a pattern with matches: n/N cycle with Reveal
        tv.search.start_input();
        for c in "foo".chars() {
            tv.search_event(&ts::key(KeyCode::Char(c)), &lines, 0);
        }
        tv.search_event(&ts::key(KeyCode::Enter), &lines, 0);
        assert_eq!(
            tv.search_event(&ts::key(KeyCode::Char('n')), &lines, 0),
            SearchOutcome::Reveal(2)
        );
        assert_eq!(
            tv.search_event(&ts::key(KeyCode::Char('n')), &lines, 0),
            SearchOutcome::Reveal(3)
        );
        assert_eq!(
            tv.search_event(&ts::key(KeyCode::Char('N')), &lines, 0),
            SearchOutcome::Reveal(2)
        );
        // Ctrl+n is not search cycling
        let ctrl_n = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(tv.search_event(&ctrl_n, &lines, 0), SearchOutcome::Ignored);
    }

    #[test]
    fn alt_modified_chars_are_not_search_cycling() {
        // macOS Option+n reports Char('n')+ALT; with a confirmed search
        // active it must not jump to the next match
        let mut tv = TextView::new();
        let lines: Vec<&str> = LINES.to_vec();
        tv.search.start_input();
        for c in "foo".chars() {
            tv.search_event(&ts::key(KeyCode::Char(c)), &lines, 0);
        }
        tv.search_event(&ts::key(KeyCode::Enter), &lines, 0);
        assert_eq!(tv.search.match_count(), 3);
        let alt_n = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('n'),
            KeyModifiers::ALT,
        ));
        assert_eq!(tv.search_event(&alt_n, &lines, 0), SearchOutcome::Ignored);
        let alt_big_n = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('N'),
            KeyModifiers::ALT,
        ));
        assert_eq!(
            tv.search_event(&alt_big_n, &lines, 0),
            SearchOutcome::Ignored
        );
        // plain n still cycles
        assert_eq!(
            tv.search_event(&ts::key(KeyCode::Char('n')), &lines, 0),
            SearchOutcome::Reveal(2)
        );
    }

    #[test]
    fn layout_splits_footer_and_clamps() {
        let mut tv = TextView::new();
        let inner = Rect::new(1, 1, 20, 10);
        // no search: the area is returned whole; scroll is clamped
        tv.scroll.set(95);
        let (content, footer, scroll) = tv.layout(inner, 100);
        assert_eq!(content, inner);
        assert_eq!(footer, None);
        assert_eq!(scroll, 90);
        assert_eq!(tv.scroll.get(), 90);
        // hscroll clamped against the widest line and written back
        tv.max_width.set(30);
        tv.hscroll.set(99);
        let (_, _, _) = tv.layout(inner, 100);
        assert_eq!(tv.hscroll.get(), 10);
        // active search carves off the footer row
        tv.search.start_input();
        let (content, footer, _) = tv.layout(inner, 100);
        assert_eq!(content, Rect::new(1, 1, 20, 9));
        assert_eq!(footer, Some(Rect::new(1, 10, 20, 1)));
    }

    #[test]
    fn layout_with_cursor_follows_the_cursor() {
        let tv = TextView::new();
        let inner = Rect::new(0, 0, 20, 10);
        // cursor below the window pulls the scroll down
        tv.scroll.set(0);
        let (_, _, scroll) = tv.layout_with_cursor(inner, 100, 15);
        assert_eq!(scroll, 6);
        // cursor above the window pulls the scroll up
        let (_, _, scroll) = tv.layout_with_cursor(inner, 100, 2);
        assert_eq!(scroll, 2);
        // cursor inside the window: scroll unchanged
        tv.scroll.set(5);
        let (_, _, scroll) = tv.layout_with_cursor(inner, 100, 7);
        assert_eq!(scroll, 5);
        // clamped against the content end
        let (_, _, scroll) = tv.layout_with_cursor(inner, 100, 99);
        assert_eq!(scroll, 90);
        assert_eq!(tv.scroll.get(), 90);
    }

    #[test]
    fn reset_clears_everything() {
        let mut tv = TextView::new();
        tv.scroll.set(5);
        tv.hscroll.set(8);
        tv.max_width.set(30);
        tv.search.start_input();
        tv.search_event(&ts::key(KeyCode::Char('f')), &LINES, 0);
        tv.reset();
        assert_eq!(tv.scroll.get(), 0);
        assert_eq!(tv.hscroll.get(), 0);
        assert_eq!(tv.max_width.get(), 0);
        assert!(!tv.search.is_active());
    }

    #[test]
    fn draw_search_footer_draws_status_text() {
        let mut tv = TextView::new();
        tv.search.start_input();
        for c in "foo".chars() {
            tv.search_event(&ts::key(KeyCode::Char(c)), &LINES, 0);
        }
        let theme = Theme::default();
        let t = ts::render(30, 2, |f| {
            tv.draw_search_footer(f, Rect::new(0, 1, 30, 1), &theme);
        });
        let s = ts::dump(&t);
        assert!(s.contains("/foo  [1/3]"), "{s}");
        // styled with theme.info
        let buf = t.backend().buffer();
        assert_eq!(buf[(0, 1)].fg, theme.info.fg.unwrap());
    }
}
