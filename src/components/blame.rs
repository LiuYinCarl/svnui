//! Blame popup: shows `svn blame` output with per-revision coloring.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::InternalEvent;
use crate::svn::models::BlameLine;
use crate::ui::{self, style::Theme};
use crossterm::event::{Event, KeyCode};
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
}

impl BlamePopup {
    pub fn new(ctx: &Context, path: &str) -> Self {
        Self {
            ctx: ctx.clone(),
            path: path.to_string(),
            lines: Vec::new(),
            pending: true,
            scroll: Cell::new(0),
        }
    }

    pub fn update(&mut self, lines: Vec<BlameLine>) {
        self.pending = false;
        self.lines = lines;
        self.scroll.set(0);
    }

    pub fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        let len = self.lines.len();
        let mut scroll = self.scroll.get();
        if key_match(k, KeyAction::MoveDown) {
            scroll = scroll.saturating_add(1);
        } else if key_match(k, KeyAction::MoveUp) {
            scroll = scroll.saturating_sub(1);
        } else if key_match(k, KeyAction::PageDown) {
            scroll = scroll.saturating_add(20);
        } else if key_match(k, KeyAction::PageUp) {
            scroll = scroll.saturating_sub(20);
        } else if key_match(k, KeyAction::Home) {
            scroll = 0;
        } else if key_match(k, KeyAction::End) {
            scroll = len;
        } else if key_match(k, KeyAction::ClosePopup) || key_match(k, KeyAction::Quit) {
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        } else if k.code == KeyCode::Char('?') {
            self.ctx.queue.push(InternalEvent::OpenHelp);
            return Ok(EventState::consumed());
        } else {
            return Ok(EventState::not_consumed());
        }
        self.scroll.set(scroll.min(len));
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
        let total = self.lines.len();
        let scroll = ui::clamp_scroll(self.scroll.get(), total, inner.height as usize);
        self.scroll.set(scroll);
        let end = (scroll + inner.height as usize).min(total);
        let mut lines: Vec<Line> = Vec::with_capacity(end - scroll);
        for bl in &self.lines[scroll..end] {
            lines.push(blame_line(bl, theme));
        }
        ui::render_lines(f, inner, &lines, 0, &[]);
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        self.event(ev)
    }
}

fn blame_line(bl: &BlameLine, theme: &Theme) -> Line<'static> {
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
    spans.push(Span::raw(bl.content.clone()));
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
        assert_eq!(b.scroll.get(), 1);
        b.event(&ts::key(crossterm::event::KeyCode::Char('G')))
            .unwrap();
        assert_eq!(b.scroll.get(), 3);
        b.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(b.scroll.get(), 3); // bounded
        b.event(&ts::key(crossterm::event::KeyCode::Char('g')))
            .unwrap();
        assert_eq!(b.scroll.get(), 0);
        b.event(&ts::key(crossterm::event::KeyCode::PageDown))
            .unwrap();
        b.event(&ts::key(crossterm::event::KeyCode::PageUp))
            .unwrap();
        // q closes
        b.event(&ts::key(crossterm::event::KeyCode::Char('q')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
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
    }
}
