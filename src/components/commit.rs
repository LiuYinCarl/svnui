//! Bottom commit bar: single-line message input.
//!
//! Text editing is delegated to the `tui-textarea` crate (the same approach
//! gitui uses), which handles wide characters (CJK), grapheme-aware deletion
//! and horizontal scrolling — so the cursor stays aligned with mixed
//! Chinese/ASCII input.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::{ConfirmAction, InternalEvent};
use crossterm::event::{Event, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use tui_textarea::{CursorMove, CursorRenderMode, TextArea};

pub struct CommitComponent {
    pub ctx: Context,
    /// Text editor (unicode-width aware cursor & grapheme-aware editing)
    pub textarea: TextArea<'static>,
    pub focused: bool,
    /// Status hint, e.g. "3 staged · commit all".
    pub hint: String,
}

impl CommitComponent {
    pub fn new(ctx: &Context) -> Self {
        let mut textarea = TextArea::new(vec![String::new()]);
        textarea.set_style(ctx.theme.text);
        textarea.set_cursor_style(Style::default().bg(ctx.theme.selection_bg));
        textarea.set_cursor_render_mode(CursorRenderMode::Hidden);
        Self {
            ctx: ctx.clone(),
            textarea,
            focused: false,
            hint: String::new(),
        }
    }

    pub fn focus(&mut self) {
        self.focused = true;
        self.textarea.set_cursor_render_mode(CursorRenderMode::Cell);
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
        self.textarea
            .set_cursor_render_mode(CursorRenderMode::Hidden);
    }

    /// Current commit message (single line).
    pub fn text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Clear the input (after a successful commit).
    pub fn clear(&mut self) {
        self.textarea.set_lines(vec![String::new()], (0, 0));
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

        // The textarea draws its own (unicode-width aware) cursor cell.
        f.render_widget(&self.textarea, inner);

        // hint line below (only if the bar is tall enough)
        if area.height >= 4 {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
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
                            message: self.text(),
                            paths: Vec::new(),
                        }));
                    return Ok(EventState::consumed());
                }
                self.textarea.insert_char(c);
            }
            KeyCode::Enter => {
                self.ctx
                    .queue
                    .push(InternalEvent::Confirm(ConfirmAction::Commit {
                        message: self.text(),
                        paths: Vec::new(),
                    }));
            }
            KeyCode::Backspace => {
                self.textarea.delete_char();
            }
            KeyCode::Left => self.textarea.move_cursor(CursorMove::Back),
            KeyCode::Right => self.textarea.move_cursor(CursorMove::Forward),
            KeyCode::Home => self.textarea.move_cursor(CursorMove::Head),
            KeyCode::End => self.textarea.move_cursor(CursorMove::End),
            KeyCode::Esc => {
                self.unfocus();
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
    use crate::ui::style::Theme;
    use ratatui::style::Color;

    fn ev(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    fn comp() -> (CommitComponent, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        (CommitComponent::new(&ctx), q)
    }

    fn type_text(c: &mut CommitComponent, s: &str) {
        for ch in s.chars() {
            c.event(&ev(KeyCode::Char(ch))).unwrap();
        }
    }

    /// Render the component and locate the cursor cell (the cell carrying the
    /// selection-bg cursor style drawn by tui-textarea).
    fn cursor_cell_x(c: &CommitComponent, w: u16, h: u16) -> u16 {
        let theme = &c.ctx.theme;
        let t = ts::render(w, h, |f| {
            c.draw(f, Rect::new(0, 0, w, h)).unwrap();
        });
        let buf = t.backend().buffer();
        for y in 0..h {
            for x in 0..w {
                if buf[(x, y)].bg == theme.selection_bg {
                    return x;
                }
            }
        }
        panic!("cursor cell not found in buffer");
    }

    #[test]
    fn typing_and_editing() {
        let (mut c, _q) = comp();
        c.focus();
        type_text(&mut c, "fix: bug");
        assert_eq!(c.text(), "fix: bug");
        // move left 3 and insert
        for _ in 0..3 {
            c.event(&ev(KeyCode::Left)).unwrap();
        }
        c.event(&ev(KeyCode::Char('X'))).unwrap();
        assert_eq!(c.text(), "fix: Xbug");
        // backspace removes the X
        c.event(&ev(KeyCode::Backspace)).unwrap();
        assert_eq!(c.text(), "fix: bug");
        // home/end move the cursor
        c.event(&ev(KeyCode::Home)).unwrap();
        c.event(&ev(KeyCode::Char('!'))).unwrap();
        assert_eq!(c.text(), "!fix: bug");
        c.event(&ev(KeyCode::End)).unwrap();
        c.event(&ev(KeyCode::Char('!'))).unwrap();
        assert_eq!(c.text(), "!fix: bug!");
    }

    /// Regression test for the reported bug: with Chinese (double-width)
    /// input the terminal cursor must land on the correct *cell*, not on the
    /// character index.
    #[test]
    fn cjk_cursor_aligns_to_cells() {
        let (mut c, _q) = comp();
        c.focus();
        type_text(&mut c, "中文ab");
        // move cursor 2 left → after "中文"
        c.event(&ev(KeyCode::Left)).unwrap();
        c.event(&ev(KeyCode::Left)).unwrap();
        // screen x = border(1) + cells of "中文"(4) = 5
        assert_eq!(
            cursor_cell_x(&c, 30, 3),
            5,
            "cursor after '中文' must be at cell 5"
        );
        // type another char: it lands between 中文 and ab
        c.event(&ev(KeyCode::Char('c'))).unwrap();
        assert_eq!(c.text(), "中文cab");
        // cursor now after "中文c": cells = 4 + 1 → x = 6
        assert_eq!(cursor_cell_x(&c, 30, 3), 6);
        // end of line: cells = width("中文cab") = 2+2+1+1+1 = 7 → x = 8
        c.event(&ev(KeyCode::End)).unwrap();
        assert_eq!(cursor_cell_x(&c, 30, 3), 8);
    }

    /// Mixed ascii + CJK: cursor after "ab" (ascii) vs after "中文".
    #[test]
    fn cjk_cursor_mixed_width() {
        let (mut c, _q) = comp();
        c.focus();
        type_text(&mut c, "ab中文");
        // cursor at end: cells = 1+1+2+2 = 6 → x = 7
        assert_eq!(cursor_cell_x(&c, 30, 3), 7);
        // backspace removes "文" → "ab中"; cursor after 中:
        // x = border(1) + cells("ab中" = 4) = 5
        c.event(&ev(KeyCode::Backspace)).unwrap();
        assert_eq!(c.text(), "ab中");
        assert_eq!(cursor_cell_x(&c, 30, 3), 5);
    }

    /// A single CJK char is one scalar: one backspace removes it entirely
    /// (this is what matters for Chinese commit messages).
    #[test]
    fn backspace_deletes_whole_cjk_char() {
        let (mut c, _q) = comp();
        c.focus();
        type_text(&mut c, "中a");
        c.event(&ev(KeyCode::Backspace)).unwrap();
        assert_eq!(c.text(), "中");
        c.event(&ev(KeyCode::Backspace)).unwrap();
        assert_eq!(c.text(), "");
    }

    /// Multi-scalar emoji clusters delete scalar-by-scalar in this version
    /// of tui-textarea; ensure repeated backspaces never panic or corrupt.
    #[test]
    fn backspace_handles_multi_scalar_emoji() {
        let (mut c, _q) = comp();
        c.focus();
        type_text(&mut c, "👨‍👩‍👧");
        assert_eq!(c.text().chars().count(), 5); // 3 emoji scalars + 2 ZWJ
        for _ in 0..5 {
            c.event(&ev(KeyCode::Backspace)).unwrap();
        }
        assert_eq!(c.text(), "");
    }

    #[test]
    fn enter_pushes_commit_confirm() {
        let (mut c, q) = comp();
        c.focus();
        type_text(&mut c, "修复 bug");
        c.event(&ev(KeyCode::Enter)).unwrap();
        match q.pop() {
            Some(InternalEvent::Confirm(ConfirmAction::Commit { message, paths })) => {
                assert_eq!(message, "修复 bug");
                assert!(paths.is_empty());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn esc_unfocuses_and_unfocused_ignores() {
        let (mut c, _q) = comp();
        assert!(!c.focused);
        // unfocused: 'q' is not consumed (so the app can quit)
        let state = c.event(&ev(KeyCode::Char('q'))).unwrap();
        assert!(!state.consumed);
        c.focus();
        // focused: 'q' is typed as text
        c.event(&ev(KeyCode::Char('q'))).unwrap();
        assert_eq!(c.text(), "q");
        c.event(&ev(KeyCode::Esc)).unwrap();
        assert!(!c.focused);
        // unfocus hides the cursor render mode again
        assert!(matches!(
            c.textarea.cursor_render_mode(),
            CursorRenderMode::Hidden
        ));
    }

    #[test]
    fn clear_resets_text_and_cursor() {
        let (mut c, _q) = comp();
        c.focus();
        type_text(&mut c, "some message");
        assert_eq!(c.text(), "some message");
        c.clear();
        assert_eq!(c.text(), "");
        assert_eq!(c.textarea.cursor(), (0, 0));
    }

    #[test]
    fn draw_shows_text_and_hint() {
        let (mut c, _q) = comp();
        c.focus();
        type_text(&mut c, "hello");
        c.hint = "2 staged".to_string();
        let terminal = ts::render(60, 5, |f| {
            c.draw(f, Rect::new(0, 0, 60, 5)).unwrap();
        });
        let s = ts::dump(&terminal);
        assert!(s.contains("hello"), "{s}");
        assert!(s.contains("Commit message"), "{s}");
        assert!(s.contains("2 staged"), "{s}");
    }

    #[test]
    fn cjk_text_renders_wide_in_buffer() {
        let (mut c, _q) = comp();
        c.focus();
        type_text(&mut c, "中文");
        let t = ts::render(30, 3, |f| {
            c.draw(f, Rect::new(0, 0, 30, 3)).unwrap();
        });
        let buf = t.backend().buffer();
        // 中 starts at cell (1,1) and spans 2 cells; 文 at (3,1)
        assert_eq!(buf[(1, 1)].symbol(), "中");
        assert_eq!(buf[(2, 1)].symbol(), " ");
        assert_eq!(buf[(3, 1)].symbol(), "文");
        assert_eq!(buf[(4, 1)].symbol(), " ");
        // cursor cell (after 中文) carries the selection bg at x = 1 + 4
        assert_eq!(buf[(5, 1)].bg, Color::Rgb(0x3b, 0x42, 0x61));
    }
}
