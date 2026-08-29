//! Blame popup: shows `svn blame` output with per-revision coloring.

use super::text_search::{InputOutcome, TextSearch, highlight_spans};
use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::InternalEvent;
use crate::svn::models::BlameLine;
use crate::ui::{self, style::Theme};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear};
use std::cell::Cell;

pub struct BlamePopup {
    ctx: Context,
    pub path: String,
    pub lines: Vec<BlameLine>,
    pub pending: bool,
    scroll: Cell<usize>,
    /// Cursor: index of the highlighted line; Enter opens that line's
    /// revision diff. `scroll` is derived from it at draw time.
    selected: Cell<usize>,
    /// Horizontal scroll offset in display columns (`h`/`l`)
    hscroll: Cell<usize>,
    /// Display width of the widest rendered line (for clamping)
    max_width: Cell<usize>,
    /// Incremental search over the blame content (`/`, n/N)
    pub search: TextSearch,
}

impl BlamePopup {
    pub fn new(ctx: &Context, path: &str) -> Self {
        Self {
            ctx: ctx.clone(),
            path: path.to_string(),
            lines: Vec::new(),
            pending: true,
            scroll: Cell::new(0),
            selected: Cell::new(0),
            hscroll: Cell::new(0),
            max_width: Cell::new(0),
            search: TextSearch::new(),
        }
    }

    pub fn update(&mut self, lines: Vec<BlameLine>) {
        use unicode_width::UnicodeWidthStr;
        self.pending = false;
        // rendered line: 7-col revision + space + author + 2 spaces + content
        self.max_width.set(
            lines
                .iter()
                .map(|l| {
                    10 + UnicodeWidthStr::width(l.author.as_str())
                        + UnicodeWidthStr::width(l.content.as_str())
                })
                .max()
                .unwrap_or(0),
        );
        self.lines = lines;
        self.scroll.set(0);
        self.selected.set(0);
        self.hscroll.set(0);
        self.search.clear();
    }

    /// Recompute search matches against the blame content and move the
    /// cursor to the (new) current match.
    fn refresh_search(&mut self) {
        let lines: Vec<&str> = self.lines.iter().map(|l| l.content.as_str()).collect();
        self.search.recompute(&lines, self.selected.get());
        if let Some(line) = self.search.current_match_line() {
            self.selected.set(line);
        }
    }

    pub fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        // Esc: cancel search input / clear search highlights first; only
        // with no active search does it close the popup.
        if let Event::Key(k) = ev
            && key_match(k, KeyAction::ClosePopup)
        {
            if self.search.is_input_mode() {
                self.search.cancel();
            } else if self.search.is_active() {
                self.search.clear();
            } else {
                self.ctx.queue.push(InternalEvent::ClosePopup);
            }
            return Ok(EventState::consumed());
        }
        // search input mode: everything goes into the pattern
        if self.search.is_input_mode() {
            if self.search.input_event(ev) == InputOutcome::Changed {
                self.refresh_search();
            }
            return Ok(EventState::consumed());
        }
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        let len = self.lines.len();
        let mut selected = self.selected.get();
        if key_match(k, KeyAction::MoveDown) {
            selected = selected.saturating_add(1);
        } else if key_match(k, KeyAction::MoveUp) {
            selected = selected.saturating_sub(1);
        } else if key_match(k, KeyAction::PageDown) {
            selected = selected.saturating_add(20);
        } else if key_match(k, KeyAction::PageUp) {
            selected = selected.saturating_sub(20);
        } else if key_match(k, KeyAction::Home) {
            selected = 0;
        } else if key_match(k, KeyAction::End) {
            selected = len.saturating_sub(1);
        } else if key_match(k, KeyAction::MoveLeft) {
            self.hscroll.set(self.hscroll.get().saturating_sub(8));
            return Ok(EventState::consumed());
        } else if key_match(k, KeyAction::MoveRight) {
            // right bound is applied at draw time (needs the area width)
            self.hscroll.set(self.hscroll.get().saturating_add(8));
            return Ok(EventState::consumed());
        } else if key_match(k, KeyAction::Enter) {
            // jump to the diff of the revision that last touched this line
            if let Some(bl) = self.lines.get(selected) {
                match bl.revision {
                    Some(rev) => self.ctx.queue.push(InternalEvent::RequestRevisionDiff(rev)),
                    None => self.ctx.queue.push(InternalEvent::ShowInfoMsg(
                        "line is not committed yet (no revision)".to_string(),
                    )),
                }
            }
            return Ok(EventState::consumed());
        } else if key_match(k, KeyAction::Filter) {
            self.search.start_input();
            return Ok(EventState::consumed());
        } else if k.code == KeyCode::Char('n') && !k.modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(line) = self.search.next_match() {
                self.selected.set(line);
            }
            return Ok(EventState::consumed());
        } else if k.code == KeyCode::Char('N') && !k.modifiers.contains(KeyModifiers::CONTROL) {
            if let Some(line) = self.search.prev_match() {
                self.selected.set(line);
            }
            return Ok(EventState::consumed());
        } else if key_match(k, KeyAction::Quit) {
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        } else if k.code == KeyCode::Char('?') {
            self.ctx.queue.push(InternalEvent::OpenHelp);
            return Ok(EventState::consumed());
        } else {
            return Ok(EventState::not_consumed());
        }
        self.selected.set(ui::clamp_index(selected, len));
        Ok(EventState::consumed())
    }
}

impl DrawableComponent for BlamePopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(format!(
                "{}: {}",
                crate::strings::TITLE.blame,
                ui::truncate(&self.path, 60)
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);

        if self.pending {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled("Loading...", theme.dim))),
                inner,
            );
            return Ok(());
        }
        if self.lines.is_empty() {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                    "No blame info",
                    theme.dim,
                ))),
                inner,
            );
            return Ok(());
        }

        // Virtualized rendering: only build the visible window of lines.
        // While a search is active the bottom row is a `/pattern` footer.
        let (inner, footer) = if self.search.is_active() && inner.height > 1 {
            (
                Rect::new(inner.x, inner.y, inner.width, inner.height - 1),
                Some(Rect::new(
                    inner.x,
                    inner.y + inner.height - 1,
                    inner.width,
                    1,
                )),
            )
        } else {
            (inner, None)
        };
        let total = self.lines.len();
        let selected = ui::clamp_index(self.selected.get(), total);
        self.selected.set(selected);
        // keep the cursor line inside the visible window
        let height = inner.height as usize;
        let mut scroll = ui::clamp_scroll(self.scroll.get(), total, height);
        if selected < scroll {
            scroll = selected;
        } else if height > 0 && selected >= scroll + height {
            scroll = selected + 1 - height;
        }
        let scroll = ui::clamp_scroll(scroll, total, height);
        self.scroll.set(scroll);
        let end = (scroll + height).min(total);
        let mut lines: Vec<Line> = Vec::with_capacity(end - scroll);
        for i in scroll..end {
            let (ranges, current) = self.search.line_ranges(i);
            lines.push(blame_line(&self.lines[i], theme, &ranges, current));
        }
        // clamp the horizontal offset against the widest line
        let h_off = self
            .hscroll
            .get()
            .min(self.max_width.get().saturating_sub(inner.width as usize));
        self.hscroll.set(h_off);
        // `lines` is the pre-sliced window, so the highlight index is
        // relative to it
        let highlights = [(selected - scroll, Style::default().bg(theme.selection_bg))];
        ui::render_lines_h(f, inner, &lines, 0, &highlights, h_off);
        if let Some(footer) = footer {
            let line = Line::from(Span::styled(self.search.status_text(), theme.info));
            f.buffer_mut()
                .set_line(footer.x, footer.y, &line, footer.width);
        }
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        self.event(ev)
    }
}

fn blame_line(
    bl: &BlameLine,
    theme: &Theme,
    matches: &[(usize, usize)],
    current: Option<usize>,
) -> Line<'static> {
    let mut spans = Vec::new();
    match bl.revision {
        Some(rev) => {
            let style = theme.blame_rev_alt[(rev as usize) % theme.blame_rev_alt.len()];
            spans.push(Span::styled(format!("{rev:>7}"), style));
        }
        None => {
            spans.push(Span::styled("      -", theme.dim));
        }
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(bl.author.clone(), theme.blame_author));
    spans.push(Span::raw("  "));
    // search hits inside the content get the hit styles
    spans.extend(highlight_spans(
        &bl.content,
        Style::default(),
        matches,
        current,
        theme.search_hit,
        theme.search_hit_current,
    ));
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::InternalEvent;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn line(rev: Option<u64>, author: &str, content: &str) -> BlameLine {
        BlameLine {
            revision: rev,
            author: author.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn update_and_scroll() {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut b = BlamePopup::new(&ctx, "src/main.rs");
        assert!(b.pending);
        b.update(vec![
            line(Some(1), "a", "x"),
            line(None, "-", "y"),
            line(Some(3), "b", "z"),
        ]);
        assert!(!b.pending);
        b.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(b.selected.get(), 1);
        b.event(&ts::key(crossterm::event::KeyCode::Char('G')))
            .unwrap();
        assert_eq!(b.selected.get(), 2);
        b.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(b.selected.get(), 2); // bounded
        b.event(&ts::key(crossterm::event::KeyCode::Char('g')))
            .unwrap();
        assert_eq!(b.selected.get(), 0);
        b.event(&ts::key(crossterm::event::KeyCode::PageDown))
            .unwrap();
        assert_eq!(b.selected.get(), 2);
        b.event(&ts::key(crossterm::event::KeyCode::PageUp))
            .unwrap();
        assert_eq!(b.selected.get(), 0);
        // q closes
        b.event(&ts::key(crossterm::event::KeyCode::Char('q')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }

    #[test]
    fn enter_opens_revision_diff_of_cursor_line() {
        let (mut b, q) = blame_with_lines();
        // cursor starts on line 0, whose revision is 1
        b.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestRevisionDiff(1))
        ));
        // move the cursor down, Enter follows it
        b.event(&ts::key(KeyCode::Char('j'))).unwrap();
        b.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestRevisionDiff(2))
        ));
        // uncommitted lines have no revision to jump to
        let (mut b2, q2) = {
            let q = crate::queue::Queue::new();
            let ctx = Context {
                queue: q.clone(),
                theme: Theme::default(),
            };
            let mut b = BlamePopup::new(&ctx, "f");
            b.update(vec![line(None, "-", "uncommitted")]);
            (b, q)
        };
        b2.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(q2.pop(), Some(InternalEvent::ShowInfoMsg(_))));
    }

    #[test]
    fn horizontal_scroll_clamps_at_draw() {
        let (mut b, _q) = blame_with_lines();
        // h saturates at 0, l moves right by 8 columns
        b.event(&ts::key(KeyCode::Char('h'))).unwrap();
        assert_eq!(b.hscroll.get(), 0);
        b.event(&ts::key(KeyCode::Char('l'))).unwrap();
        assert_eq!(b.hscroll.get(), 8);
        // draw clamps against the widest line: inner width is 22 here
        ts::render(24, 8, |f| {
            b.draw(f, Rect::new(0, 0, 24, 8)).unwrap();
        });
        assert_eq!(b.hscroll.get(), b.max_width.get() - 22);
        // new blame data resets the offset
        b.update(vec![line(Some(1), "a", "x")]);
        assert_eq!(b.hscroll.get(), 0);
        assert_eq!(b.max_width.get(), 12);
    }

    fn blame_with_lines() -> (BlamePopup, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut b = BlamePopup::new(&ctx, "src/main.rs");
        b.update(
            (1..=20)
                .map(|i| {
                    let content = if i == 8 || i == 15 {
                        format!("needle line {i}")
                    } else {
                        format!("plain line {i}")
                    };
                    line(Some(i), "alice", &content)
                })
                .collect(),
        );
        (b, q)
    }

    #[test]
    fn search_highlights_scrolls_and_cycles() {
        let (mut b, q) = blame_with_lines();
        b.event(&ts::key(KeyCode::Char('/'))).unwrap();
        assert!(b.search.is_input_mode());
        for c in "needle".chars() {
            b.event(&ts::key(KeyCode::Char(c))).unwrap();
        }
        // live: both matches found, cursor on the first (line index 7)
        assert_eq!(b.search.match_count(), 2);
        assert_eq!(b.selected.get(), 7);
        let t = ts::render(60, 8, |f| {
            b.draw(f, Rect::new(0, 0, 60, 8)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("/needle  [1/2]"), "{s}");
        assert!(s.contains("needle line 8"), "{s}");
        // Enter keeps highlights; n/N cycle with wrapping
        b.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(!b.search.is_input_mode());
        assert!(b.search.is_active());
        b.event(&ts::key(KeyCode::Char('n'))).unwrap();
        assert_eq!(b.selected.get(), 14);
        assert_eq!(b.search.status_text(), "/needle  [2/2]");
        b.event(&ts::key(KeyCode::Char('n'))).unwrap();
        assert_eq!(b.selected.get(), 7); // wrapped
        b.event(&ts::key(KeyCode::Char('N'))).unwrap();
        assert_eq!(b.selected.get(), 14);
        // scroll keys still work with highlights active
        b.event(&ts::key(KeyCode::Char('g'))).unwrap();
        assert_eq!(b.selected.get(), 0);
        // first Esc clears highlights, second closes
        b.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(!b.search.is_active());
        assert!(q.pop().is_none());
        b.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }

    #[test]
    fn esc_in_input_cancels_and_q_types_into_pattern() {
        let (mut b, q) = blame_with_lines();
        b.event(&ts::key(KeyCode::Char('/'))).unwrap();
        // 'q' is pattern text while typing, not "close"
        b.event(&ts::key(KeyCode::Char('q'))).unwrap();
        assert_eq!(b.search.pattern(), "q");
        assert!(q.pop().is_none());
        // backspace edits the pattern
        b.event(&ts::key(KeyCode::Backspace)).unwrap();
        assert_eq!(b.search.pattern(), "");
        b.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(!b.search.is_active());
        assert!(q.pop().is_none());
        // q outside input mode still closes
        b.event(&ts::key(KeyCode::Char('q'))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }

    #[test]
    fn search_highlights_stay_aligned_with_hscroll() {
        // search active + horizontal scroll at the same time: the match
        // highlight must follow the sliced text, not the pre-slice columns
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut b = BlamePopup::new(&ctx, "f");
        b.update(vec![
            line(Some(1), "al", "xx needle yy plus some more text"),
            line(Some(2), "al", "plain"),
        ]);
        b.event(&ts::key(KeyCode::Char('/'))).unwrap();
        for c in "needle".chars() {
            b.event(&ts::key(KeyCode::Char(c))).unwrap();
        }
        b.event(&ts::key(KeyCode::Enter)).unwrap();
        assert_eq!(b.search.match_count(), 1);
        b.event(&ts::key(KeyCode::Char('l'))).unwrap();
        assert_eq!(b.hscroll.get(), 8);
        let theme = Theme::default();
        let t = ts::render(24, 6, |f| {
            b.draw(f, Rect::new(0, 0, 24, 6)).unwrap();
        });
        let buf = t.backend().buffer();
        // line prefix is rev(7) + ' ' + author(2) + 2 spaces = 12 columns;
        // skipping 8 leaves 4, so 'needle' (content column 3..9) lands at
        // screen x = 1 + 4 + 3 = 8. The single match is the current one.
        let hit_bg = theme.search_hit_current.bg.unwrap();
        for x in 8..14 {
            assert_eq!(buf[(x, 1)].bg, hit_bg, "cell {x} must be highlighted");
        }
        assert_ne!(buf[(7, 1)].bg, hit_bg, "the match must not shift left");
        assert_eq!(buf[(8, 1)].symbol(), "n");
        let s = ts::dump(&t);
        assert!(s.contains("/needle  [1/1]"), "{s}");
    }

    #[test]
    fn draw_pending_empty_and_lines() {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut b = BlamePopup::new(&ctx, "src/main.rs");
        let t1 = ts::render(80, 10, |f| {
            b.draw(f, Rect::new(0, 0, 80, 10)).unwrap();
        });
        assert!(ts::dump(&t1).contains("Loading"));

        b.update(vec![]);
        let t2 = ts::render(80, 10, |f| {
            b.draw(f, Rect::new(0, 0, 80, 10)).unwrap();
        });
        assert!(ts::dump(&t2).contains("No blame"));

        b.update(vec![
            line(Some(42), "kenshin", "fn main() {"),
            line(None, "-", "  todo"),
        ]);
        let t3 = ts::render(80, 10, |f| {
            b.draw(f, Rect::new(0, 0, 80, 10)).unwrap();
        });
        let s = ts::dump(&t3);
        assert!(s.contains("Blame: src/main.rs"), "{s}");
        assert!(s.contains("42"), "{s}");
        assert!(s.contains("kenshin"), "{s}");
        assert!(s.contains("fn main() {"), "{s}");
        assert!(s.contains("todo"), "{s}");
        // the cursor line (first line, row 1 inside the border) is
        // highlighted with the selection background
        let buf = t3.backend().buffer();
        let sel = ratatui::style::Color::Rgb(0x3b, 0x42, 0x61);
        assert!((0..80).any(|x| buf[(x, 1)].bg == sel));
    }
}
