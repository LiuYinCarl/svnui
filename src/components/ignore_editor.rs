//! Ignore tab: a full-screen `.svnignore` editor with live syntax checking.
//!
//! The file lives at the working-copy root and filters the status tree
//! (see `crate::ignore_filter`). Editing is delegated to a multi-line
//! `tui-textarea` (the same crate as the commit input); every edit
//! re-validates all lines with the `ignore` crate's gitignore parser and
//! reports problems in a footer (`line 3: <error>`), so a broken rule is
//! caught before it is saved.
//!
//! Keys: Ctrl+s saves (and triggers a status refresh so the new filter
//! applies immediately), F5 reloads from disk — refused while there are
//! unsaved edits — Esc returns to the status tab. Everything else is text
//! input, which is why the global tab-switch digits (1/2/3/4) do not work
//! while editing (same trade-off as the commit input).

use super::{Context, DrawableComponent, EventState};
use crate::ignore_filter::IGNORE_FILE;
use crate::keys::{KeyAction, key_match};
use crate::queue::{InternalEvent, Tab};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::path::PathBuf;
use tui_textarea::{CursorRenderMode, TextArea};

/// Status-bar shortcut hints shown while the ignore tab is active.
pub const HINTS: &str = "ctrl+s save  F5 reload  esc back";

/// One syntax problem found while validating the current content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxIssue {
    /// 1-based line number
    pub line: usize,
    pub message: String,
}

pub struct IgnoreEditor {
    ctx: Context,
    /// `<working copy root>/.svnignore`
    path: PathBuf,
    pub textarea: TextArea<'static>,
    /// Content differs from what is on disk
    dirty: bool,
    /// Live syntax-check results, recomputed on every edit
    issues: Vec<SyntaxIssue>,
    /// Transient save feedback shown in the footer block title (a modal
    /// popup would eat the next keypress); cleared on the next edit
    notice: Option<String>,
}

impl IgnoreEditor {
    pub fn new(ctx: &Context, root: std::path::PathBuf) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_style(ctx.theme.text);
        textarea.set_cursor_style(Style::default().bg(ctx.theme.selection_bg));
        // a full-screen editor always shows its cursor (unlike the commit
        // bar, which only does while focused)
        textarea.set_cursor_render_mode(CursorRenderMode::Cell);
        let mut editor = Self {
            ctx: ctx.clone(),
            path: root.join(IGNORE_FILE),
            textarea,
            dirty: false,
            issues: Vec::new(),
            notice: None,
        };
        editor.reload();
        editor
    }

    /// The file being edited.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn issues(&self) -> &[SyntaxIssue] {
        &self.issues
    }

    /// Current editor content (trailing newline stripped).
    pub fn content(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Reload the file from disk. Refused while there are unsaved edits:
    /// silently discarding them would be worse than a stale view.
    pub fn reload(&mut self) {
        if self.dirty {
            self.ctx.queue.push(InternalEvent::ShowInfoMsg(format!(
                "{IGNORE_FILE} has unsaved edits — save with Ctrl+s first"
            )));
            return;
        }
        let content = std::fs::read_to_string(&self.path).unwrap_or_default();
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        self.textarea = TextArea::new(lines);
        self.textarea.set_style(self.ctx.theme.text);
        self.textarea
            .set_cursor_style(Style::default().bg(self.ctx.theme.selection_bg));
        self.textarea.set_cursor_render_mode(CursorRenderMode::Cell);
        self.notice = None;
        self.validate();
    }

    /// Re-validate every line with the gitignore parser.
    fn validate(&mut self) {
        self.issues = validate_lines(self.textarea.lines());
    }

    /// Write the file and ask the app to refresh the status view so the
    /// new filter takes effect. Saving with syntax issues is allowed (the
    /// broken lines simply match nothing), but the confirmation message
    /// points them out.
    fn save(&mut self) {
        let content = self.content();
        let result = if content.is_empty() {
            // an empty ruleset is the same as no file at all
            std::fs::remove_file(&self.path).or_else(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(e)
                }
            })
        } else {
            std::fs::write(&self.path, format!("{content}\n"))
        };
        match result {
            Ok(()) => {
                self.dirty = false;
                self.ctx.queue.push(InternalEvent::RefreshStatus);
                self.notice = Some(if self.issues.is_empty() {
                    "saved · status filter refreshed".to_string()
                } else {
                    format!(
                        "saved ({} line(s) with syntax issues — they match nothing)",
                        self.issues.len()
                    )
                });
            }
            Err(e) => self.ctx.queue.push(InternalEvent::ShowInfoMsg(format!(
                "cannot save {IGNORE_FILE}: {e}"
            ))),
        }
    }
}

/// Validate `.svnignore` content line by line with the gitignore parser;
/// blank lines and comments are always fine. Two classes of issues are
/// reported: hard parse errors (dangling `\`, reversed `[z-a]` ranges —
/// the line then matches nothing) and trailing-whitespace warnings
/// (gitignore strips unescaped trailing blanks, so `"foo "` silently
/// matches `"foo"` — almost never the intent).
fn validate_lines(lines: &[String]) -> Vec<SyntaxIssue> {
    let mut issues = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let mut builder = ignore::gitignore::GitignoreBuilder::new("");
        if let Err(e) = builder.add_line(None, line) {
            issues.push(SyntaxIssue {
                line: i + 1,
                message: e.to_string(),
            });
            continue;
        }
        let trimmed = line.trim_end_matches([' ', '\t']);
        if trimmed.len() != line.len() && !trimmed.is_empty() && !trimmed.starts_with('#') {
            // the whitespace is only kept when escaped; whether the '\'
            // right before it escapes or is itself escaped depends on the
            // parity of the backslash run ("foo\ " keeps the space,
            // "foo\\ " does not)
            let backslashes = trimmed.chars().rev().take_while(|&c| c == '\\').count();
            if backslashes % 2 == 0 {
                issues.push(SyntaxIssue {
                    line: i + 1,
                    message:
                        "trailing whitespace is ignored by the parser (escape it with \\ to match it)"
                            .to_string(),
                });
            }
        }
    }
    issues
}

impl DrawableComponent for IgnoreEditor {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        // footer: syntax issues (up to a few) or a quiet "no issues" line
        let footer_h = if self.issues.is_empty() {
            3
        } else {
            (self.issues.len() as u16 + 2).min(8)
        };
        let chunks =
            Layout::vertical([Constraint::Min(3), Constraint::Length(footer_h)]).split(area);

        let dirty_mark = if self.dirty { " ●" } else { "" };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused))
            .title(format!("{}{dirty_mark}", self.path.display()));
        f.render_widget(&self.textarea, block.inner(chunks[0]));
        f.render_widget(block, chunks[0]);

        let footer_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_unfocused))
            .title(match &self.notice {
                Some(n) => format!("Syntax check — {n}"),
                None => "Syntax check".to_string(),
            });
        let footer_inner = footer_block.inner(chunks[1]);
        f.render_widget(footer_block, chunks[1]);
        if self.issues.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled("no issues", theme.dim))),
                footer_inner,
            );
        } else {
            let lines: Vec<Line> = self
                .issues
                .iter()
                .take(footer_inner.height as usize)
                .map(|i| {
                    Line::from(Span::styled(
                        format!("line {}: {}", i.line, i.message),
                        theme.status_deleted,
                    ))
                })
                .collect();
            f.render_widget(Paragraph::new(lines), footer_inner);
        }
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        // bracketed paste: insert verbatim
        if let Event::Paste(text) = ev {
            self.textarea.insert_str(text);
            self.dirty = true;
            self.notice = None;
            self.validate();
            return Ok(EventState::consumed());
        }
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::IgnoreSave) {
            self.save();
        } else if k.code == KeyCode::F(5) {
            // NB: not KeyAction::Refresh — that action also matches a plain
            // 'R', which must stay text input here (patterns like "README"
            // would trigger a reload instead of typing)
            self.reload();
        } else if key_match(k, KeyAction::Escape) {
            // the only way out: text input consumes every other key,
            // including the global tab-switch digits
            self.ctx.queue.push(InternalEvent::SwitchTab(Tab::Status));
        } else {
            // ignore control/alt combos (Ctrl+c must not insert 'c'), same
            // guard as the commit input
            if k.modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && matches!(k.code, KeyCode::Char(_))
            {
                return Ok(EventState::consumed());
            }
            self.textarea.input(*k);
            self.dirty = true;
            self.notice = None;
            self.validate();
        }
        Ok(EventState::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Queue;
    use crate::test_support as ts;
    use crate::ui::style::Theme;
    use crossterm::event::KeyCode;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A temp dir as the fake working-copy root, cleaned up on drop.
    struct Root(std::path::PathBuf);

    impl Root {
        fn new() -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("svnui-editor-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn editor(root: &Root) -> (IgnoreEditor, Queue) {
        let q = Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        (IgnoreEditor::new(&ctx, root.0.clone()), q)
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    fn type_text(e: &mut IgnoreEditor, s: &str) {
        for ch in s.chars() {
            let code = match ch {
                '\n' => KeyCode::Enter,
                c => KeyCode::Char(c),
            };
            e.event(&key(code)).unwrap();
        }
    }

    #[test]
    fn missing_file_starts_empty_and_save_creates_it() {
        let root = Root::new();
        let (mut e, q) = editor(&root);
        assert_eq!(e.content(), "");
        assert!(!e.is_dirty());
        type_text(&mut e, "*.log\nbuild/");
        assert!(e.is_dirty());
        assert!(e.issues().is_empty());
        e.event(&Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('s'),
            crossterm::event::KeyModifiers::CONTROL,
        )))
        .unwrap();
        assert!(!e.is_dirty());
        assert_eq!(
            std::fs::read_to_string(root.0.join(IGNORE_FILE)).unwrap(),
            "*.log\nbuild/\n"
        );
        // saving asks the app to re-filter the status view
        let events = q.drain();
        assert!(
            events
                .iter()
                .any(|ev| matches!(ev, InternalEvent::RefreshStatus)),
            "{events:?}"
        );
    }

    #[test]
    fn invalid_lines_are_reported_live() {
        let root = Root::new();
        let (mut e, _q) = editor(&root);
        // a dangling backslash is a hard parse error (matches nothing)
        type_text(&mut e, "*.log\nfoo\\\nok/\n");
        assert_eq!(e.issues().len(), 1);
        assert_eq!(e.issues()[0].line, 2);
        assert!(
            e.issues()[0].message.contains("dangling"),
            "{:?}",
            e.issues()
        );
        // deleting the backslash fixes the line (cursor starts on the
        // trailing empty line 4, so two Ups reach line 2)
        e.event(&key(KeyCode::Up)).unwrap();
        e.event(&key(KeyCode::Up)).unwrap();
        e.event(&key(KeyCode::End)).unwrap();
        e.event(&key(KeyCode::Backspace)).unwrap();
        assert!(e.issues().is_empty());
    }

    #[test]
    fn trailing_whitespace_warns() {
        let root = Root::new();
        let (mut e, _q) = editor(&root);
        // unescaped trailing blanks are silently stripped by the parser
        type_text(&mut e, "foo ");
        assert_eq!(e.issues().len(), 1);
        assert!(e.issues()[0].message.contains("trailing whitespace"));
        // escaped trailing blank is intentional, no warning
        let root2 = Root::new();
        let (mut e2, _q) = editor(&root2);
        type_text(&mut e2, "foo\\ ");
        assert!(e2.issues().is_empty());
        // but "foo\\ " (escaped backslash, then a blank) does NOT keep the
        // blank — the escape is consumed by the second backslash, so warn
        let root3 = Root::new();
        let (mut e3, _q) = editor(&root3);
        type_text(&mut e3, "foo\\\\ ");
        assert_eq!(e3.issues().len(), 1);
    }

    /// Regression test: 'R' is KeyAction::Refresh elsewhere, but here it
    /// must be plain text input — a pattern like "README" must not trigger
    /// a reload (and its unsaved-edits popup).
    #[test]
    fn capital_r_is_text_input_not_reload() {
        let root = Root::new();
        let (mut e, q) = editor(&root);
        type_text(&mut e, "README");
        assert_eq!(e.content(), "README");
        assert!(q.pop().is_none(), "no reload attempt, no popup");
    }

    #[test]
    fn reload_is_blocked_while_dirty() {
        let root = Root::new();
        std::fs::write(root.0.join(IGNORE_FILE), "*.log\n").unwrap();
        let (mut e, q) = editor(&root);
        assert_eq!(e.content(), "*.log");
        type_text(&mut e, "\ntmp/");
        // external change arrives; F5 must not discard the edits
        std::fs::write(root.0.join(IGNORE_FILE), "external\n").unwrap();
        e.event(&key(KeyCode::F(5))).unwrap();
        assert!(e.content().contains("tmp/"), "edits must survive");
        assert!(matches!(q.pop(), Some(InternalEvent::ShowInfoMsg(_))));
        // after saving, reload picks up external content again
        e.event(&Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('s'),
            crossterm::event::KeyModifiers::CONTROL,
        )))
        .unwrap();
        std::fs::write(root.0.join(IGNORE_FILE), "external\n").unwrap();
        e.event(&key(KeyCode::F(5))).unwrap();
        assert_eq!(e.content(), "external");
    }

    #[test]
    fn esc_returns_to_status_tab() {
        let root = Root::new();
        let (mut e, q) = editor(&root);
        e.event(&key(KeyCode::Esc)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::SwitchTab(Tab::Status))
        ));
        // plain digits are text input, not tab switches
        e.event(&key(KeyCode::Char('1'))).unwrap();
        assert_eq!(e.content(), "1");
    }

    #[test]
    fn empty_content_removes_the_file() {
        let root = Root::new();
        std::fs::write(root.0.join(IGNORE_FILE), "*.log\n").unwrap();
        let (mut e, _q) = editor(&root);
        // delete all lines
        loop {
            let before = e.content();
            e.textarea.delete_line_by_end();
            e.textarea.delete_line_by_head();
            if e.textarea.lines().len() == 1 && e.textarea.lines()[0].is_empty() {
                break;
            }
            if e.content() == before && before.is_empty() {
                break;
            }
            e.event(&key(KeyCode::Backspace)).ok();
            if e.content().is_empty() {
                break;
            }
        }
        assert_eq!(e.content(), "");
        e.event(&Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('s'),
            crossterm::event::KeyModifiers::CONTROL,
        )))
        .unwrap();
        assert!(!root.0.join(IGNORE_FILE).exists());
    }

    #[test]
    fn draw_shows_issues_and_dirty_marker() {
        let root = Root::new();
        let (mut e, _q) = editor(&root);
        type_text(&mut e, "foo\\\n");
        let t = ts::render(70, 12, |f| {
            e.draw(f, Rect::new(0, 0, 70, 12)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains(IGNORE_FILE), "{s}");
        assert!(s.contains('●'), "{s}");
        assert!(s.contains("line 1:"), "{s}");
        assert!(s.contains("Syntax check"), "{s}");
        // clean content: quiet footer
        let root2 = Root::new();
        let (mut e2, _q) = editor(&root2);
        type_text(&mut e2, "*.log");
        let t = ts::render(70, 12, |f| {
            e2.draw(f, Rect::new(0, 0, 70, 12)).unwrap();
        });
        assert!(ts::dump(&t).contains("no issues"));
    }
}
