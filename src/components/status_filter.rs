//! Status file filter popup: a small input dialog for the status tab.
//!
//! Like the commit search popup, this popup is input-only — the results
//! appear in the file tree behind it: typing live-filters the status
//! entries, Enter keeps the filter, Esc restores the filter text the
//! popup was opened with.

use super::{Context, DrawableComponent, EventState};
use crate::queue::InternalEvent;
use crate::ui::{self, style::Theme};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear};

pub struct StatusFilterPopup {
    ctx: Context,
    query: String,
    /// The filter text the popup was opened with; Esc restores it so the
    /// tree is not left filtered by an abandoned half-typed query
    initial: String,
}

impl StatusFilterPopup {
    pub fn new(ctx: &Context, initial: &str) -> Self {
        Self {
            ctx: ctx.clone(),
            query: initial.to_string(),
            initial: initial.to_string(),
        }
    }

    fn push_input(&self) {
        self.ctx
            .queue
            .push(InternalEvent::StatusFilterInput(self.query.clone()));
    }
}

impl DrawableComponent for StatusFilterPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme: &Theme = &self.ctx.theme;
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(crate::strings::TITLE.status_filter);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(inner);

        ui::render_line_at(
            f,
            chunks[0].x,
            chunks[0].y,
            chunks[0].width,
            &Line::from(vec![
                Span::styled("> ", theme.info),
                Span::raw(self.query.clone()),
            ]),
        );
        ui::render_line_at(
            f,
            chunks[1].x,
            chunks[1].y,
            chunks[1].width,
            &Line::from(Span::styled(
                "Typing filters the status list · Enter: apply · Esc: cancel",
                theme.dim,
            )),
        );
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        if let Event::Paste(text) = ev {
            self.query.push_str(text);
            self.push_input();
            return Ok(EventState::consumed());
        }
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        match k.code {
            KeyCode::Esc => {
                // restore the filter the popup was opened with before
                // closing: the live-typed text is abandoned
                self.ctx
                    .queue
                    .push(InternalEvent::StatusFilterInput(self.initial.clone()));
                self.ctx.queue.push(InternalEvent::ClosePopup);
            }
            KeyCode::Enter => {
                // the live input already applied the filter; just close
                self.ctx.queue.push(InternalEvent::ClosePopup);
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.push_input();
            }
            KeyCode::Char(c)
                if !k
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(c);
                self.push_input();
            }
            _ => return Ok(EventState::not_consumed()),
        }
        Ok(EventState::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn comp(initial: &str) -> (StatusFilterPopup, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        (StatusFilterPopup::new(&ctx, initial), q)
    }

    #[test]
    fn typing_live_filters_and_enter_keeps_the_filter() {
        let (mut p, q) = comp("");
        for ch in "rs".chars() {
            p.event(&ts::key(KeyCode::Char(ch))).unwrap();
            assert!(matches!(q.pop(), Some(InternalEvent::StatusFilterInput(_))));
        }
        p.event(&ts::key(KeyCode::Backspace)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::StatusFilterInput(s)) if s == "r"
        ));
        // Enter keeps the live-pushed filter and closes (no extra event)
        p.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(q.pop().is_none());
    }

    #[test]
    fn esc_restores_the_initial_filter() {
        let (mut p, q) = comp("saved");
        for ch in "xy".chars() {
            p.event(&ts::key(KeyCode::Char(ch))).unwrap();
            assert!(matches!(q.pop(), Some(InternalEvent::StatusFilterInput(_))));
        }
        p.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::StatusFilterInput(s)) if s == "saved"
        ));
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(q.pop().is_none());
    }

    #[test]
    fn paste_and_ctrl_guard_and_draw() {
        let (mut p, q) = comp("");
        // ctrl combos are not query text
        let ctrl_c = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ));
        assert!(!p.event(&ctrl_c).unwrap().consumed);
        // paste appends and live-filters
        p.event(&Event::Paste("配置文件".into())).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::StatusFilterInput(s)) if s == "配置文件"
        ));
        let t = ts::render(70, 8, |f| {
            p.draw(f, Rect::new(0, 0, 70, 8)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("Filter status files"), "{s}");
        // wide chars occupy two cells in the buffer, so match a single one
        assert!(s.contains('配'), "{s}");
        assert!(s.contains("Typing filters the status list"), "{s}");
    }
}
