//! File history popup: `svn log -v -- <path>` for a single file.
//!
//! Opened from the status tab (`t`) or from the file finder. Shows the
//! revisions that touched the file; Enter/`d` opens the fullscreen diff of
//! the selected revision, `b` opens blame for the file.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::InternalEvent;
use crate::svn::models::LogEntry;
use crate::ui::{self, style::Theme};
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear};
use std::cell::Cell;

pub struct FileLogPopup {
    ctx: Context,
    pub path: String,
    pub entries: Vec<LogEntry>,
    pub pending: bool,
    selection: usize,
    scroll: Cell<usize>,
    detail_scroll: Cell<usize>,
}

impl FileLogPopup {
    pub fn new(ctx: &Context, path: &str) -> Self {
        Self {
            ctx: ctx.clone(),
            path: path.to_string(),
            entries: Vec::new(),
            pending: true,
            selection: 0,
            scroll: Cell::new(0),
            detail_scroll: Cell::new(0),
        }
    }

    pub fn update(&mut self, entries: Vec<LogEntry>) {
        self.pending = false;
        self.entries = entries;
        self.selection = self.selection.min(self.entries.len().saturating_sub(1));
    }

    fn selection_revision(&self) -> Option<u64> {
        self.entries.get(self.selection).map(|e| e.revision)
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        self.selection = ui::clamp_index((self.selection as isize + delta).max(0) as usize, len);
        self.detail_scroll.set(0);
    }

    /// Scroll the commit-message pane, clamped to the content length.
    fn scroll_detail(&mut self, delta: isize) {
        let len = self
            .entries
            .get(self.selection)
            .map(|e| e.message.lines().count().max(1))
            .unwrap_or(0);
        let next =
            (self.detail_scroll.get() as isize + delta).clamp(0, len.saturating_sub(1) as isize);
        self.detail_scroll.set(next as usize);
    }
}

impl DrawableComponent for FileLogPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(format!(
                "{}: {}",
                crate::strings::TITLE.file_history,
                ui::truncate(&self.path, 50)
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);

        if self.pending {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                    crate::strings::MSG.loading,
                    theme.dim,
                ))),
                inner,
            );
            return Ok(());
        }
        if self.entries.is_empty() {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                    crate::strings::MSG.empty_log,
                    theme.dim,
                ))),
                inner,
            );
            return Ok(());
        }

        // top: revision list; bottom: message of the selected revision
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(inner);

        let lines: Vec<Line> = self
            .entries
            .iter()
            .map(|e| revision_line(e, theme))
            .collect();
        let view_h = chunks[0].height as usize;
        let scroll = ui::scroll_follow(self.selection, self.scroll.get(), lines.len(), view_h);
        self.scroll.set(scroll);
        let highlights = vec![(self.selection, Style::default().bg(theme.selection_bg))];
        ui::render_lines(f, chunks[0], &lines, scroll, &highlights);

        if let Some(e) = self.entries.get(self.selection) {
            let mut detail: Vec<Line> = Vec::new();
            for line in e.message.lines() {
                detail.push(Line::from(Span::styled(
                    line.to_string(),
                    theme.log_message,
                )));
            }
            if detail.is_empty() {
                detail.push(Line::from(Span::styled("(no message)", theme.dim)));
            }
            let dscroll = ui::clamp_scroll(
                self.detail_scroll.get(),
                detail.len(),
                chunks[1].height as usize,
            );
            self.detail_scroll.set(dscroll);
            ui::render_lines(f, chunks[1], &detail, dscroll, &[]);
        }
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::ClosePopup) || key_match(k, KeyAction::Quit) {
            self.ctx.queue.push(InternalEvent::ClosePopup);
        } else if key_match(k, KeyAction::MoveDown) {
            self.move_selection(1);
        } else if key_match(k, KeyAction::MoveUp) {
            self.move_selection(-1);
        } else if key_match(k, KeyAction::PageDown) {
            self.move_selection(20);
        } else if key_match(k, KeyAction::PageUp) {
            self.move_selection(-20);
        } else if key_match(k, KeyAction::Home) {
            self.selection = 0;
            self.detail_scroll.set(0);
        } else if key_match(k, KeyAction::End) {
            self.selection = self.entries.len().saturating_sub(1);
            self.detail_scroll.set(0);
        } else if key_match(k, KeyAction::DetailScrollDown) {
            self.scroll_detail(10);
        } else if key_match(k, KeyAction::DetailScrollUp) {
            self.scroll_detail(-10);
        } else if key_match(k, KeyAction::OpenRevisionDiff) {
            if let Some(rev) = self.selection_revision() {
                self.ctx.queue.push(InternalEvent::RequestRevisionDiff(rev));
            }
        } else if key_match(k, KeyAction::Blame) {
            self.ctx
                .queue
                .push(InternalEvent::RequestBlame(self.path.clone()));
        } else if key_match(k, KeyAction::Help) {
            self.ctx.queue.push(InternalEvent::OpenHelp);
        } else {
            return Ok(EventState::not_consumed());
        }
        Ok(EventState::consumed())
    }
}

fn revision_line(e: &LogEntry, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("r{:<7}", e.revision), theme.log_revision),
        Span::styled(
            format!("{:<10}", ui::truncate(&e.author, 10)),
            theme.log_author,
        ),
        Span::raw(" "),
        Span::styled(ui::truncate(&e.summary(), 60), theme.log_message),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn entry(rev: u64, msg: &str) -> LogEntry {
        LogEntry {
            revision: rev,
            author: "alice".into(),
            date: "2026-01-01".into(),
            line_count: 1,
            changed: vec![('M', "src/main.rs".into())],
            message: msg.into(),
        }
    }

    fn comp() -> (FileLogPopup, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut p = FileLogPopup::new(&ctx, "src/main.rs");
        p.update(vec![entry(5, "five"), entry(2, "two\nsecond line")]);
        (p, q)
    }

    #[test]
    fn navigation_and_revision_diff() {
        let (mut p, q) = comp();
        assert_eq!(p.selection_revision(), Some(5));
        p.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(p.selection_revision(), Some(2));
        p.event(&ts::key(crossterm::event::KeyCode::End)).unwrap();
        assert_eq!(p.selection_revision(), Some(2));
        p.event(&ts::key(crossterm::event::KeyCode::Home)).unwrap();
        assert_eq!(p.selection_revision(), Some(5));
        p.event(&ts::key(crossterm::event::KeyCode::PageDown))
            .unwrap();
        p.event(&ts::key(crossterm::event::KeyCode::PageUp))
            .unwrap();
        assert_eq!(p.selection_revision(), Some(5));
        // Enter requests the revision diff
        p.event(&ts::key(crossterm::event::KeyCode::Enter)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestRevisionDiff(5))
        ));
        // 'd' also requests it
        p.event(&ts::key(crossterm::event::KeyCode::Char('d')))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestRevisionDiff(5))
        ));
        // 'b' requests blame for the file (regardless of selection)
        p.event(&ts::key(crossterm::event::KeyCode::Char('b')))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestBlame(path)) if path == "src/main.rs"
        ));
        // '?' opens help, 'q' closes
        p.event(&ts::key(crossterm::event::KeyCode::Char('?')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::OpenHelp)));
        p.event(&ts::key(crossterm::event::KeyCode::Char('q')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        // unknown keys are not consumed
        assert!(
            !p.event(&ts::key(crossterm::event::KeyCode::Char('z')))
                .unwrap()
                .consumed
        );
    }

    #[test]
    fn ctrl_d_u_scroll_the_message_pane() {
        let (mut p, _q) = comp();
        let ctrl_d = crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        let ctrl_u = crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('u'),
            crossterm::event::KeyModifiers::CONTROL,
        ));
        // selection r5, message "five": 1 line — nothing to scroll
        p.event(&ctrl_d).unwrap();
        assert_eq!(p.detail_scroll.get(), 0);
        // r2's message has two lines: Ctrl+d scrolls, clamped to len - 1
        p.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(p.selection_revision(), Some(2));
        p.event(&ctrl_d).unwrap();
        assert_eq!(p.detail_scroll.get(), 1);
        p.event(&ctrl_u).unwrap();
        assert_eq!(p.detail_scroll.get(), 0);
    }

    #[test]
    fn draw_states() {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut p = FileLogPopup::new(&ctx, "src/main.rs");
        // pending
        let t1 = ts::render(80, 12, |f| {
            p.draw(f, Rect::new(0, 0, 80, 12)).unwrap();
        });
        assert!(ts::dump(&t1).contains("Loading"));
        // empty
        p.update(vec![]);
        let t2 = ts::render(80, 12, |f| {
            p.draw(f, Rect::new(0, 0, 80, 12)).unwrap();
        });
        assert!(ts::dump(&t2).contains("No revisions"));
        // entries + message detail
        p.update(vec![entry(5, "fix the bug"), entry(2, "")]);
        let t3 = ts::render(80, 12, |f| {
            p.draw(f, Rect::new(0, 0, 80, 12)).unwrap();
        });
        let s = ts::dump(&t3);
        assert!(s.contains("File history: src/main.rs"), "{s}");
        assert!(s.contains("r5"), "{s}");
        assert!(s.contains("alice"), "{s}");
        assert!(s.contains("fix the bug"), "{s}");
    }
}
