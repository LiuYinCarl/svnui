//! Help popup: lists all keybindings.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, all_bindings, key_match};
use crate::queue::InternalEvent;
use crate::ui;
use crossterm::event::{Event, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear};
use std::cell::Cell;

pub struct HelpPopup {
    ctx: Context,
    scroll: Cell<usize>,
}

impl HelpPopup {
    pub fn new(ctx: &Context) -> Self {
        Self {
            ctx: ctx.clone(),
            scroll: Cell::new(0),
        }
    }
}

impl DrawableComponent for HelpPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        // clear the cells behind the popup so the underlying tab content
        // does not bleed through
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(crate::strings::TITLE.help);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(
            "svnui — an SVN TUI inspired by gitui",
            theme.title_focused,
        )));
        lines.push(Line::from(""));
        for b in all_bindings() {
            lines.push(Line::from(vec![
                Span::styled(format!("  {: <18}", b.keys), theme.info),
                Span::styled(b.description, theme.text),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Press q or Esc to close",
            theme.dim,
        )));

        let scroll = ui::clamp_scroll(self.scroll.get(), lines.len(), inner.height as usize);
        self.scroll.set(scroll);
        ui::render_lines(f, inner, &lines, scroll, &[]);
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::ClosePopup)
            || key_match(k, KeyAction::Quit)
            || k.code == KeyCode::Char('?')
        {
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        }
        let mut scroll = self.scroll.get();
        if key_match(k, KeyAction::MoveDown) {
            scroll += 1;
        } else if key_match(k, KeyAction::MoveUp) {
            scroll = scroll.saturating_sub(1);
        } else if key_match(k, KeyAction::PageDown) {
            scroll += 20;
        } else if key_match(k, KeyAction::PageUp) {
            scroll = scroll.saturating_sub(20);
        } else {
            return Ok(EventState::not_consumed());
        }
        self.scroll.set(scroll);
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
    fn draw_and_close() {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut h = HelpPopup::new(&ctx);
        let t = ts::render(80, 30, |f| {
            h.draw(f, Rect::new(0, 0, 80, 30)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("svnui"), "{s}");
        assert!(s.contains("Quit svnui"), "{s}");
        assert!(s.contains("Help"), "{s}");
        // scroll down
        h.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(h.scroll.get(), 1);
        // q closes
        h.event(&ts::key(crossterm::event::KeyCode::Char('q')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        // '?' also closes
        h.event(&ts::key(crossterm::event::KeyCode::Char('?')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }
}
