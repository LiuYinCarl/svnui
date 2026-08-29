//! Bottom commit bar: single-line message input.
//!
//! Text editing is delegated to the `tui-textarea` crate (the same approach
//! gitui uses), which handles wide characters (CJK), grapheme-aware deletion
//! and horizontal scrolling — so the cursor stays aligned with mixed
//! Chinese/ASCII input.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::{ConfirmAction, InternalEvent};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use tui_textarea::{CursorMove, CursorRenderMode, TextArea};

/// How many recent commit messages the Tab picker lists.
pub const HISTORY_PICKER_LEN: usize = 10;

pub struct CommitComponent {
    pub ctx: Context,
    /// Text editor (unicode-width aware cursor & grapheme-aware editing)
    pub textarea: TextArea<'static>,
    pub focused: bool,
    /// Status hint, e.g. "3 staged · commit all".
    pub hint: String,
    /// Recent commit messages (newest first), shown by the Tab picker
    history: Vec<String>,
    /// Whether the history picker is open
    history_open: bool,
    /// Selected row in the history picker
    history_sel: usize,
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
            history: Vec::new(),
            history_open: false,
            history_sel: 0,
        }
    }

    pub fn focus(&mut self) {
        self.focused = true;
        self.textarea.set_cursor_render_mode(CursorRenderMode::Cell);
    }

    pub fn unfocus(&mut self) {
        self.focused = false;
        self.history_open = false;
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

    /// Replace the recent-message list (called when the log refreshes).
    pub fn set_history(&mut self, messages: Vec<String>) {
        self.history = messages.into_iter().take(HISTORY_PICKER_LEN).collect();
        if self.history_sel >= self.history.len() {
            self.history_sel = self.history.len().saturating_sub(1);
        }
    }

    fn open_history_picker(&mut self) {
        if !self.history.is_empty() {
            self.history_open = true;
            self.history_sel = 0;
        }
    }

    /// Fill the input with the selected history message and close the picker.
    fn apply_history_selection(&mut self) {
        if let Some(msg) = self.history.get(self.history_sel) {
            self.textarea.set_lines(vec![msg.clone()], (0, 0));
            self.textarea.move_cursor(CursorMove::End);
        }
        self.history_open = false;
    }

    /// Handle a key while the history picker is open. Returns false when
    /// the picker is closed and normal editing should proceed.
    fn handle_picker_key(&mut self, k: &crossterm::event::KeyEvent) -> bool {
        if !self.history_open {
            return false;
        }
        let len = self.history.len();
        match k.code {
            KeyCode::Up => {
                self.history_sel = self.history_sel.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Tab => {
                self.history_sel = (self.history_sel + 1).min(len.saturating_sub(1));
            }
            KeyCode::Enter => {
                self.apply_history_selection();
            }
            KeyCode::Esc => {
                self.history_open = false;
            }
            _ => {}
        }
        true
    }

    /// Draw the recent-messages picker floating above the commit bar.
    fn draw_history_picker(&self, f: &mut Frame, area: Rect, theme: &crate::ui::style::Theme) {
        let rows = self.history.len().min(HISTORY_PICKER_LEN) as u16;
        let height = (rows + 2).min(area.y); // keep it above the bar
        if height < 3 {
            return;
        }
        let rect = Rect::new(area.x, area.y - height, area.width, height);
        f.render_widget(Clear, rect);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title("Recent commit messages");
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        let visible = (height - 2) as usize;
        // keep the selection inside the visible window on short terminals
        let offset = if self.history_sel >= visible {
            self.history_sel - visible + 1
        } else {
            0
        };
        let lines: Vec<Line> = self
            .history
            .iter()
            .skip(offset)
            .take(visible)
            .map(|m| Line::from(Span::raw(m.clone())))
            .collect();
        let highlights = vec![(
            self.history_sel - offset,
            Style::default()
                .bg(theme.selection_bg)
                .add_modifier(Modifier::BOLD),
        )];
        crate::ui::render_lines(f, inner, &lines, 0, &highlights);
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
                    format!(
                        "[Enter] commit · [Tab] history · [Esc] cancel   {}",
                        self.hint
                    ),
                    theme.dim,
                ))),
                Rect::new(inner.x, inner.y + inner.height - 1, inner.width, 1),
            );
        }

        // history picker: a floating list right above the commit bar
        if self.history_open && !self.history.is_empty() {
            self.draw_history_picker(f, area, theme);
        }
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        if !self.focused {
            return Ok(EventState::not_consumed());
        }
        // bracketed paste: insert the pasted text verbatim (never triggers
        // commit, even when the text contains newlines)
        if let Event::Paste(text) = ev {
            self.textarea.insert_str(text);
            return Ok(EventState::consumed());
        }
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        // the history picker is modal while open
        if self.handle_picker_key(k) {
            return Ok(EventState::consumed());
        }
        // Tab opens the recent-messages picker (Shift+Tab still cycles focus)
        if k.code == KeyCode::Tab && !k.modifiers.contains(KeyModifiers::SHIFT) {
            self.open_history_picker();
            return Ok(EventState::consumed());
        }
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
                // ignore control/alt combos (Ctrl+C must not insert 'c')
                if k.modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    return Ok(EventState::not_consumed());
                }
                self.textarea.insert_char(c);
            }
            KeyCode::Enter => {
                // same check as the Char branch: Shift+Enter must not commit
                if key_match(k, KeyAction::CommitConfirm) {
                    self.ctx
                        .queue
                        .push(InternalEvent::Confirm(ConfirmAction::Commit {
                            message: self.text(),
                            paths: Vec::new(),
                        }));
                }
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
    fn modifier_keys_do_not_insert_or_commit() {
        let (mut c, q) = comp();
        c.focus();
        // Ctrl+C is not text input
        let ctrl_c = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let state = c.event(&ctrl_c).unwrap();
        assert!(!state.consumed);
        assert_eq!(c.text(), "");
        // Shift+Enter does not commit
        let shift_enter = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::SHIFT,
        ));
        let state = c.event(&shift_enter).unwrap();
        assert!(state.consumed);
        assert!(q.pop().is_none(), "Shift+Enter must not commit");
        // plain Enter still commits
        c.event(&ev(KeyCode::Enter)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Confirm(ConfirmAction::Commit { .. }))
        ));
    }

    #[test]
    fn ctrl_s_pushes_commit_confirm() {
        let (mut c, q) = comp();
        c.focus();
        type_text(&mut c, "fix something");
        let ctrl_s = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('s'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let state = c.event(&ctrl_s).unwrap();
        assert!(state.consumed);
        match q.pop() {
            Some(InternalEvent::Confirm(ConfirmAction::Commit { message, paths })) => {
                assert_eq!(message, "fix something");
                assert!(paths.is_empty());
            }
            other => panic!("unexpected {other:?}"),
        }
        // the 's' was not inserted as text
        assert_eq!(c.text(), "fix something");
    }

    #[test]
    fn history_picker_swallows_char_keys() {
        let (mut c, q) = comp();
        c.focus();
        c.set_history(vec!["one".to_string(), "two".to_string()]);
        c.event(&ev(KeyCode::Tab)).unwrap();
        // while the picker is open, plain chars neither insert text nor
        // reach the commit shortcuts — the picker is modal
        let state = c.event(&ev(KeyCode::Char('x'))).unwrap();
        assert!(state.consumed);
        assert_eq!(c.text(), "");
        let ctrl_s = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('s'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let state = c.event(&ctrl_s).unwrap();
        assert!(state.consumed);
        assert!(q.pop().is_none(), "picker must not leak a commit event");
        // after Esc closes the picker, typing works again
        c.event(&ev(KeyCode::Esc)).unwrap();
        c.event(&ev(KeyCode::Char('x'))).unwrap();
        assert_eq!(c.text(), "x");
    }

    #[test]
    fn set_history_truncates_to_picker_len() {
        let (mut c, _q) = comp();
        c.set_history((0..25).map(|i| format!("m{i}")).collect());
        assert_eq!(c.history.len(), HISTORY_PICKER_LEN);
        assert_eq!(c.history.last().unwrap(), "m9");
        // a stale selection beyond the new (shorter) history is clamped
        c.focus();
        c.event(&ev(KeyCode::Tab)).unwrap();
        for _ in 0..9 {
            c.event(&ev(KeyCode::Down)).unwrap();
        }
        assert_eq!(c.history_sel, 9);
        c.set_history(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(c.history_sel, 2);
        // Enter now fills the clamped selection, not an out-of-range one
        c.event(&ev(KeyCode::Enter)).unwrap();
        assert_eq!(c.text(), "c");
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
        // height 4 matches the production bar (StatusTab gives it
        // Constraint::Length(4)): the hint line must actually render
        let terminal = ts::render(60, 4, |f| {
            c.draw(f, Rect::new(0, 0, 60, 4)).unwrap();
        });
        let s = ts::dump(&terminal);
        assert!(s.contains("hello"), "{s}");
        assert!(s.contains("Commit message"), "{s}");
        assert!(s.contains("[Tab] history"), "{s}");
        assert!(s.contains("2 staged"), "{s}");
    }

    #[test]
    fn history_picker_keeps_selection_visible() {
        let (mut c, _q) = comp();
        c.focus();
        c.set_history((0..10).map(|i| format!("msg{i}")).collect());
        c.event(&ev(KeyCode::Tab)).unwrap();
        // select msg5; with only 3 visible rows the picker must scroll
        for _ in 0..5 {
            c.event(&ev(KeyCode::Down)).unwrap();
        }
        let terminal = ts::render(40, 8, |f| {
            c.draw(f, Rect::new(0, 5, 40, 3)).unwrap();
        });
        let s = ts::dump(&terminal);
        assert!(s.contains("msg5"), "selection must stay visible: {s}");
        assert!(s.contains("msg3"), "{s}");
        assert!(!s.contains("msg0"), "scrolled-out rows must not draw: {s}");
        assert!(!s.contains("msg2"), "scrolled-out rows must not draw: {s}");
    }

    #[test]
    fn tab_opens_history_picker_and_enter_fills() {
        let (mut c, _q) = comp();
        c.focus();
        c.set_history(vec!["fix: first".to_string(), "feat: second".to_string()]);
        // Tab opens the picker
        c.event(&ev(KeyCode::Tab)).unwrap();
        // navigate down and fill with the second message
        c.event(&ev(KeyCode::Down)).unwrap();
        c.event(&ev(KeyCode::Enter)).unwrap();
        assert_eq!(c.text(), "feat: second");
        // picker is closed: typing appends normally
        c.event(&ev(KeyCode::Char('!'))).unwrap();
        assert_eq!(c.text(), "feat: second!");
    }

    #[test]
    fn history_picker_esc_and_empty_history() {
        let (mut c, _q) = comp();
        c.focus();
        // empty history: Tab is still consumed (focus must not jump away)
        c.event(&ev(KeyCode::Tab)).unwrap();
        assert!(c.focused);
        c.set_history(vec!["only".to_string()]);
        c.event(&ev(KeyCode::Tab)).unwrap();
        // Esc closes without filling
        c.event(&ev(KeyCode::Esc)).unwrap();
        assert_eq!(c.text(), "");
        // next Esc unfocuses the input
        c.event(&ev(KeyCode::Esc)).unwrap();
        assert!(!c.focused);
    }

    #[test]
    fn history_picker_draws_above_commit_bar() {
        let (mut c, _q) = comp();
        c.focus();
        c.set_history(vec!["历史消息一".to_string(), "history two".to_string()]);
        c.event(&ev(KeyCode::Tab)).unwrap();
        let terminal = ts::render(60, 20, |f| {
            c.draw(f, Rect::new(0, 17, 60, 3)).unwrap();
        });
        let s = ts::dump(&terminal);
        assert!(s.contains("Recent commit messages"), "{s}");
        // CJK text spans two cells per char in the buffer dump, so assert
        // on the individual glyphs rather than the contiguous string
        assert!(s.contains("历"), "{s}");
        assert!(s.contains("history two"), "{s}");
    }

    #[test]
    fn paste_inserts_text_verbatim() {
        let (mut c, q) = comp();
        c.focus();
        // a pasted multi-line CJK message must be inserted as text, never
        // trigger commit (which a raw Enter key event would)
        c.event(&Event::Paste("修复：第一行\n第二行".to_string()))
            .unwrap();
        assert_eq!(c.text(), "修复：第一行\n第二行");
        assert!(q.pop().is_none(), "paste must not commit");
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
