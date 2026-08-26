//! Yes/No confirmation popup. Confirming pushes `Confirmed(action)` back
//! to the queue so the app performs the action.

use super::super::components::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::{ConfirmAction, InternalEvent};
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub struct ConfirmPopup {
    ctx: Context,
    pub action: Option<ConfirmAction>,
    pub message: String,
}

impl ConfirmPopup {
    pub fn new(ctx: &Context, message: String, action: ConfirmAction) -> Self {
        Self {
            ctx: ctx.clone(),
            action: Some(action),
            message,
        }
    }
}

impl DrawableComponent for ConfirmPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        f.render_widget(Clear, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(crate::strings::TITLE.confirm);
        let inner = block.inner(area);
        f.render_widget(block, chunks[0]);

        let msg_lines: Vec<Line> = self
            .message
            .lines()
            .map(|l| Line::from(Span::styled(l.to_string(), theme.text)))
            .collect();
        f.render_widget(Paragraph::new(msg_lines).wrap(Wrap { trim: false }), inner);

        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("[y] ", theme.confirm_yes),
                Span::styled("yes    ", theme.text),
                Span::styled("[n] ", theme.confirm_no),
                Span::styled("no", theme.text),
            ]))
            .alignment(Alignment::Center),
            Rect::new(area.x, chunks[1].y, area.width, 1),
        );
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::Confirm) {
            if let Some(action) = self.action.take() {
                self.ctx.queue.push(InternalEvent::Confirmed(action));
            }
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        }
        if key_match(k, KeyAction::Deny) || key_match(k, KeyAction::ClosePopup) {
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        }
        Ok(EventState::not_consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::ConfirmAction;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn ctx() -> (Context, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        (
            Context {
                queue: q.clone(),
                theme: Theme::default(),
            },
            q,
        )
    }

    #[test]
    fn yes_confirms_and_closes() {
        let (c, q) = ctx();
        let mut p = ConfirmPopup::new(&c, "Do it?".to_string(), ConfirmAction::Update);
        let ev = ts::key(crossterm::event::KeyCode::Char('y'));
        let state = p.event(&ev).unwrap();
        assert!(state.consumed);
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Confirmed(ConfirmAction::Update))
        ));
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }

    #[test]
    fn no_closes_without_confirmation() {
        let (c, q) = ctx();
        let mut p = ConfirmPopup::new(&c, "Do it?".to_string(), ConfirmAction::Update);
        p.event(&ts::key(crossterm::event::KeyCode::Char('n')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(q.pop().is_none());
    }

    #[test]
    fn other_keys_ignored() {
        let (c, _q) = ctx();
        let mut p = ConfirmPopup::new(&c, "Do it?".to_string(), ConfirmAction::Update);
        let state = p
            .event(&ts::key(crossterm::event::KeyCode::Char('x')))
            .unwrap();
        assert!(!state.consumed);
    }

    #[test]
    fn draw_shows_message() {
        let (c, _q) = ctx();
        let p = ConfirmPopup::new(
            &c,
            "Revert local changes?".to_string(),
            ConfirmAction::Revert(vec!["a.txt".to_string()]),
        );
        let t = ts::render(60, 10, |f| {
            p.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("Revert local changes?"), "{s}");
        assert!(s.contains("Confirm"), "{s}");
        assert!(s.contains("[y]"), "{s}");
        assert!(s.contains("[n]"), "{s}");
    }
}
