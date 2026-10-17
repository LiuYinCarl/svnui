//! Message popup for errors and info. `j/k`/PageUp/PageDown/Home/End scroll
//! long messages; any other key dismisses it.

use super::super::components::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use std::cell::Cell;

pub struct MsgPopup {
    ctx: Context,
    pub message: String,
    pub is_error: bool,
    scroll: Cell<usize>,
}

impl MsgPopup {
    pub fn new(ctx: &Context, message: String, is_error: bool) -> Self {
        Self {
            ctx: ctx.clone(),
            message,
            is_error,
            scroll: Cell::new(0),
        }
    }
}

impl DrawableComponent for MsgPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(if self.is_error {
                "Error"
            } else {
                crate::strings::TITLE.message
            });
        let inner = block.inner(area);
        f.render_widget(block, area);

        let style = if self.is_error {
            theme.error
        } else {
            theme.info
        };
        let msg_lines: Vec<Line> = self
            .message
            .lines()
            .map(|l| Line::from(Span::styled(l.to_string(), style)))
            .collect();
        let scroll = self.scroll.get().min(u16::MAX as usize) as u16;
        f.render_widget(
            Paragraph::new(msg_lines)
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0)),
            inner,
        );
        // footer hint, one row above the bottom border
        if inner.height > 0 {
            let y = inner.y + inner.height - 1;
            f.buffer_mut()
                .set_string(inner.x, y, "j/k scroll · press any key to close", theme.dim);
        }
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return if matches!(ev, Event::Mouse(_)) {
                self.ctx.queue.push(crate::queue::InternalEvent::ClosePopup);
                Ok(EventState::consumed())
            } else {
                Ok(EventState::not_consumed())
            };
        };
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
            scroll = self.message.lines().count().saturating_sub(1);
        } else {
            self.ctx.queue.push(crate::queue::InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
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
    fn any_key_closes() {
        let q = crate::queue::Queue::new();
        let c = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut p = MsgPopup::new(&c, "boom".to_string(), true);
        p.event(&ts::key(crossterm::event::KeyCode::Char('z')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        // another key event closes as well
        let mut p2 = MsgPopup::new(&c, "boom".to_string(), false);
        p2.event(&ts::key(crossterm::event::KeyCode::Enter))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        // movement keys scroll a long error instead of dismissing it
        let mut p3 = MsgPopup::new(&c, "line0\nline1\nline2".to_string(), true);
        p3.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(p3.scroll.get(), 1);
        assert!(q.pop().is_none(), "j must not close");
        p3.event(&ts::key(crossterm::event::KeyCode::Char('z')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }

    #[test]
    fn draw_error_and_info() {
        let q = crate::queue::Queue::new();
        let c = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let p = MsgPopup::new(&c, "Error text".to_string(), true);
        let t = ts::render(60, 8, |f| {
            p.draw(f, Rect::new(0, 0, 60, 8)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("Error"), "{s}");
        assert!(s.contains("Error text"), "{s}");
        // the footer is drawn inside the border, leaving the bottom row intact
        assert!(s.contains("press any key to close"), "{s}");
        let bottom = s.lines().last().unwrap();
        assert!(bottom.starts_with('└') && bottom.ends_with('┘'), "{s}");
        assert!(!bottom.contains("press any key"), "{s}");
        let p2 = MsgPopup::new(&c, "Info text".to_string(), false);
        let t2 = ts::render(60, 8, |f| {
            p2.draw(f, Rect::new(0, 0, 60, 8)).unwrap();
        });
        assert!(ts::dump(&t2).contains("Info text"));
    }
}
