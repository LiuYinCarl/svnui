//! Bottom commit bar: single-line message input like gitui's commit area.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::{ConfirmAction, InternalEvent};
use crossterm::event::{Event, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use std::cell::Cell;

pub struct CommitComponent {
    pub ctx: Context,
    /// The commit message text (single line).
    pub text: String,
    /// Cursor position in chars.
    cursor: usize,
    pub focused: bool,
    /// Status hint, e.g. "3 staged · commit all".
    pub hint: String,
    /// Where the cursor was last drawn (for terminal cursor placement).
    pub cursor_pos: Cell<(u16, u16)>,
}

impl CommitComponent {
    pub fn new(ctx: &Context) -> Self {
        Self {
            ctx: ctx.clone(),
            text: String::new(),
            cursor: 0,
            focused: false,
            hint: String::new(),
            cursor_pos: Cell::new((0, 0)),
        }
    }

    pub fn focus(&mut self) {
        self.focused = true;
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
    }

    fn insert(&mut self, c: char) {
        let mut chars: Vec<char> = self.text.chars().collect();
        let pos = self.cursor.min(chars.len());
        chars.insert(pos, c);
        self.text = chars.into_iter().collect();
        self.cursor = pos + 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut chars: Vec<char> = self.text.chars().collect();
        let pos = self.cursor.saturating_sub(1);
        if pos < chars.len() {
            chars.remove(pos);
        }
        self.text = chars.into_iter().collect();
        self.cursor = pos;
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.text.chars().count();
        let cur = self.cursor as isize + delta;
        self.cursor = cur.clamp(0, len as isize) as usize;
    }
}

impl DrawableComponent for CommitComponent {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        let border = if self.focused {
            theme.border_focused
        } else {
            theme.border_unfocused
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .title("Commit message");
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut spans = vec![Span::styled("> ", theme.info)];
        spans.push(Span::raw(self.text.clone()));
        let line = Line::from(spans);
        f.buffer_mut()
            .set_line(inner.x, inner.y, &line, inner.width);

        if self.focused {
            // 2 = "> " prefix; cursor follows the text (char index)
            let x = inner
                .x
                .saturating_add(2)
                .saturating_add(self.cursor as u16)
                .min(inner.x + inner.width - 1);
            self.cursor_pos.set((x, inner.y));
        } else {
            self.cursor_pos.set((0, 0));
        }

        // hint line below
        if inner.height >= 2 {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                    format!("[Enter] commit · [Esc] cancel   {}", self.hint),
                    theme.dim,
                ))),
                Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
            );
        }
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        if !self.focused {
            return Ok(EventState::not_consumed());
        }
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        match k.code {
            KeyCode::Char(c) => {
                if key_match(k, KeyAction::CommitConfirm) {
                    // Ctrl+s commits
                    self.ctx
                        .queue
                        .push(InternalEvent::Confirm(ConfirmAction::Commit {
                            message: self.text.clone(),
                            paths: Vec::new(),
                        }));
                    return Ok(EventState::consumed());
                }
                self.insert(c);
            }
            KeyCode::Enter => {
                self.ctx
                    .queue
                    .push(InternalEvent::Confirm(ConfirmAction::Commit {
                        message: self.text.clone(),
                        paths: Vec::new(),
                    }));
            }
            KeyCode::Backspace => self.backspace(),
            KeyCode::Left => self.move_cursor(-1),
            KeyCode::Right => self.move_cursor(1),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.text.chars().count(),
            KeyCode::Esc => {
                self.focused = false;
            }
            _ => return Ok(EventState::not_consumed()),
        }
        Ok(EventState::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{ConfirmAction, InternalEvent};
    use crate::test_support as ts;

    fn ev(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    #[test]
    fn typing_and_editing() {
        let ctx = Context {
            queue: crate::queue::Queue::new(),
            theme: crate::ui::style::Theme::default(),
        };
        let mut c = CommitComponent::new(&ctx);
        c.focus();
        for ch in "fix: bug".chars() {
            c.event(&ev(KeyCode::Char(ch))).unwrap();
        }
        assert_eq!(c.text, "fix: bug");
        // move cursor to middle and insert (cursor 5 = between ' ' and 'b')
        c.event(&ev(KeyCode::Left)).unwrap();
        c.event(&ev(KeyCode::Left)).unwrap();
        c.event(&ev(KeyCode::Left)).unwrap();
        c.event(&ev(KeyCode::Char('X'))).unwrap();
        assert_eq!(c.text, "fix: Xbug");
        assert_eq!(c.cursor, 6);
        // backspace removes before cursor
        c.event(&ev(KeyCode::Backspace)).unwrap();
        assert_eq!(c.text, "fix: bug");
        // home/end
        c.event(&ev(KeyCode::Home)).unwrap();
        assert_eq!(c.cursor, 0);
        c.event(&ev(KeyCode::End)).unwrap();
        assert_eq!(c.cursor, 8);
        // backspace at start is a no-op
        c.event(&ev(KeyCode::Home)).unwrap();
        c.event(&ev(KeyCode::Backspace)).unwrap();
        assert_eq!(c.text, "fix: bug");
    }

    #[test]
    fn enter_pushes_commit_confirm() {
        let ctx = Context {
            queue: crate::queue::Queue::new(),
            theme: crate::ui::style::Theme::default(),
        };
        let mut c = CommitComponent::new(&ctx);
        c.focus();
        c.event(&ev(KeyCode::Char('m'))).unwrap();
        c.event(&ev(KeyCode::Enter)).unwrap();
        match ctx.queue.pop() {
            Some(InternalEvent::Confirm(ConfirmAction::Commit { message, paths })) => {
                assert_eq!(message, "m");
                assert!(paths.is_empty());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn esc_unfocuses_and_unfocused_ignores() {
        let ctx = Context {
            queue: crate::queue::Queue::new(),
            theme: crate::ui::style::Theme::default(),
        };
        let mut c = CommitComponent::new(&ctx);
        assert!(!c.focused);
        // unfocused: 'q' is not consumed (so app can quit)
        let state = c.event(&ev(KeyCode::Char('q'))).unwrap();
        assert!(!state.consumed);
        c.focus();
        // focused: 'q' is typed as text
        c.event(&ev(KeyCode::Char('q'))).unwrap();
        assert_eq!(c.text, "q");
        c.event(&ev(KeyCode::Esc)).unwrap();
        assert!(!c.focused);
    }

    #[test]
    fn draw_shows_text_and_hint() {
        let ctx = Context {
            queue: crate::queue::Queue::new(),
            theme: crate::ui::style::Theme::default(),
        };
        let mut c = CommitComponent::new(&ctx);
        c.text = "hello".to_string();
        c.hint = "2 staged".to_string();
        let terminal = ts::render(60, 5, |f| {
            c.draw(f, Rect::new(0, 0, 60, 5)).unwrap();
        });
        let s = ts::dump(&terminal);
        assert!(s.contains("hello"), "{s}");
        assert!(s.contains("Commit message"), "{s}");
        assert!(s.contains("2 staged"), "{s}");
        // focused cursor position is tracked for the terminal
        c.focus();
        let _ = ts::render(60, 5, |f| {
            c.draw(f, Rect::new(0, 0, 60, 5)).unwrap();
        });
        // cursor at position 0 (text was assigned directly)
        let (x, _y) = c.cursor_pos.get();
        assert_eq!(x, 3); // border + "> " prefix + 0 chars
        // move right through the text
        for _ in 0..5 {
            c.event(&ev(KeyCode::Right)).unwrap();
        }
        let _ = ts::render(60, 5, |f| {
            c.draw(f, Rect::new(0, 0, 60, 5)).unwrap();
        });
        let (x2, _y2) = c.cursor_pos.get();
        assert_eq!(x2, 1 + 2 + 5);
    }
}
