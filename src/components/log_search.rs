//! Commit search popup: a small input dialog for the log tab.
//!
//! Unlike the file finder (which lists results inside the popup), this
//! popup is input-only — the results appear in the log list behind it:
//! typing live-filters the already-loaded revisions, and Enter runs a
//! full-history `svn log --search` so older commits can be found too.

use super::{Context, DrawableComponent, EventState};
use crate::queue::InternalEvent;
use crate::ui::{self, style::Theme};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear};

pub struct LogSearchPopup {
    ctx: Context,
    query: String,
}

impl LogSearchPopup {
    pub fn new(ctx: &Context, initial: &str) -> Self {
        Self {
            ctx: ctx.clone(),
            query: initial.to_string(),
        }
    }

    fn push_input(&self) {
        self.ctx
            .queue
            .push(InternalEvent::LogSearchInput(self.query.clone()));
    }
}

impl DrawableComponent for LogSearchPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme: &Theme = &self.ctx.theme;
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(crate::strings::TITLE.log_search);
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
                "Typing filters loaded commits · Enter: search all history · Esc: close",
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
                self.ctx.queue.push(InternalEvent::ClosePopup);
            }
            KeyCode::Enter => {
                if !self.query.is_empty() {
                    self.ctx
                        .queue
                        .push(InternalEvent::SearchLog(self.query.clone()));
                }
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

    fn comp(initial: &str) -> (LogSearchPopup, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        (LogSearchPopup::new(&ctx, initial), q)
    }

    #[test]
    fn typing_live_filters_and_enter_searches() {
        let (mut p, q) = comp("");
        for ch in "fix".chars() {
            p.event(&ts::key(KeyCode::Char(ch))).unwrap();
            assert!(matches!(q.pop(), Some(InternalEvent::LogSearchInput(_))));
        }
        p.event(&ts::key(KeyCode::Backspace)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::LogSearchInput(s)) if s == "fi"
        ));
        // Enter: full-history search, then close
        p.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::SearchLog(s)) if s == "fi"
        ));
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }

    #[test]
    fn esc_closes_and_empty_enter_just_closes() {
        let (mut p, q) = comp("prefilled");
        p.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(q.pop().is_none());

        let (mut p2, q2) = comp("");
        p2.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(q2.pop(), Some(InternalEvent::ClosePopup)));
        assert!(q2.pop().is_none());
    }

    #[test]
    fn paste_and_ctrl_guard_and_draw() {
        let (mut p, q) = comp("");
        // ctrl combos are not query text
        let ctrl_c = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ));
        p.event(&ctrl_c).unwrap();
        // paste appends and live-filters
        p.event(&Event::Paste("中文提交".into())).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::LogSearchInput(s)) if s == "中文提交"
        ));
        let t = ts::render(70, 8, |f| {
            p.draw(f, Rect::new(0, 0, 70, 8)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("Search commits"), "{s}");
        // wide chars occupy two cells in the buffer, so match a single one
        assert!(s.contains('中'), "{s}");
        assert!(s.contains("search all history"), "{s}");
    }
}
