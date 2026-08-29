//! Scrollable output viewer, used after svn update/commit/add/revert ops.

use super::super::components::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::InternalEvent;
use crate::ui;
use crossterm::event::{Event, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear};
use std::cell::Cell;

pub struct OutputPopup {
    ctx: Context,
    pub title: String,
    pub lines: Vec<Line<'static>>,
    scroll: Cell<usize>,
}

impl OutputPopup {
    pub fn new(ctx: &Context, title: String, content: &str) -> Self {
        let lines: Vec<Line> = content
            .lines()
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect();
        Self::from_lines(ctx, title, lines)
    }

    /// Build a popup from pre-styled lines (e.g. the repo-info overview).
    pub fn from_lines(ctx: &Context, title: String, lines: Vec<Line<'static>>) -> Self {
        Self {
            ctx: ctx.clone(),
            title,
            lines,
            scroll: Cell::new(0),
        }
    }
}

impl DrawableComponent for OutputPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(self.title.clone());
        let inner = block.inner(area);
        f.render_widget(block, area);

        if self.lines.is_empty() {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled("no output", theme.dim))),
                inner,
            );
            return Ok(());
        }

        let scroll = ui::clamp_scroll(self.scroll.get(), self.lines.len(), inner.height as usize);
        self.scroll.set(scroll);
        ui::render_lines(f, inner, &self.lines, scroll, &[]);
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
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
        } else if key_match(k, KeyAction::ClosePopup)
            || key_match(k, KeyAction::Quit)
            || k.code == KeyCode::Enter
        {
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        } else {
            return Ok(EventState::not_consumed());
        }
        self.scroll.set(scroll.min(len));
        Ok(EventState::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::InternalEvent;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    #[test]
    fn scroll_and_close() {
        let q = crate::queue::Queue::new();
        let c = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut p = OutputPopup::new(&c, "svn update".to_string(), "line1\nline2\nline3\n");
        assert_eq!(p.lines.len(), 3);
        p.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(p.scroll.get(), 1);
        p.event(&ts::key(crossterm::event::KeyCode::End)).unwrap();
        assert_eq!(p.scroll.get(), 3);
        p.event(&ts::key(crossterm::event::KeyCode::PageUp))
            .unwrap();
        p.event(&ts::key(crossterm::event::KeyCode::PageDown))
            .unwrap();
        p.event(&ts::key(crossterm::event::KeyCode::Char('g')))
            .unwrap();
        assert_eq!(p.scroll.get(), 0);
        // enter closes
        p.event(&ts::key(crossterm::event::KeyCode::Enter)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }

    #[test]
    fn draw_lines_and_empty() {
        let q = crate::queue::Queue::new();
        let c = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let p = OutputPopup::new(&c, "svn update".to_string(), "Updated to revision 5.");
        let t = ts::render(60, 8, |f| {
            p.draw(f, Rect::new(0, 0, 60, 8)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("svn update"), "{s}");
        assert!(s.contains("Updated to revision 5"), "{s}");

        let p2 = OutputPopup::new(&c, "empty".to_string(), "");
        let t2 = ts::render(60, 8, |f| {
            p2.draw(f, Rect::new(0, 0, 60, 8)).unwrap();
        });
        assert!(ts::dump(&t2).contains("no output"));
    }
}
