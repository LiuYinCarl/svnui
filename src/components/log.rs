//! Log tab: revision list + details (changed paths & message).

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::{ConfirmAction, InternalEvent};
use crate::strings::TITLE;
use crate::svn::models::LogEntry;
use crate::ui::{self, style::Theme};
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use std::cell::Cell;

pub struct LogComponent {
    ctx: Context,
    pub entries: Vec<LogEntry>,
    selection: usize,
    scroll: Cell<usize>,
    detail_scroll: Cell<usize>,
    pub pending: bool,
    pub focused: bool,
}

impl LogComponent {
    pub fn new(ctx: &Context) -> Self {
        Self {
            ctx: ctx.clone(),
            entries: Vec::new(),
            selection: 0,
            scroll: Cell::new(0),
            detail_scroll: Cell::new(0),
            pending: true,
            focused: true,
        }
    }

    pub fn update(&mut self, entries: Vec<LogEntry>) {
        self.pending = false;
        self.entries = entries;
        self.selection = self.selection.min(self.entries.len().saturating_sub(1));
    }

    pub fn selection_revision(&self) -> Option<u64> {
        self.entries.get(self.selection).map(|e| e.revision)
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        self.selection = ui::clamp_index((self.selection as isize + delta).max(0) as usize, len);
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::MoveUp) {
            self.move_selection(-1);
        } else if key_match(k, KeyAction::MoveDown) {
            self.move_selection(1);
        } else if key_match(k, KeyAction::PageUp) {
            self.move_selection(-20);
        } else if key_match(k, KeyAction::PageDown) {
            self.move_selection(20);
        } else if key_match(k, KeyAction::Home) {
            self.selection = 0;
        } else if key_match(k, KeyAction::End) {
            self.selection = self.entries.len().saturating_sub(1);
        } else if key_match(k, KeyAction::OpenRevisionDiff) {
            if let Some(rev) = self.selection_revision() {
                self.ctx.queue.push(InternalEvent::RequestRevisionDiff(rev));
            }
        } else if key_match(k, KeyAction::UpdateToRevision) {
            if let Some(rev) = self.selection_revision() {
                self.ctx
                    .queue
                    .push(InternalEvent::Confirm(ConfirmAction::UpdateToRevision(rev)));
            }
        } else if key_match(k, KeyAction::Refresh) {
            self.ctx
                .queue
                .push(InternalEvent::Update(crate::queue::NeedsUpdate::LOG));
        } else if key_match(k, KeyAction::Help) {
            self.ctx.queue.push(InternalEvent::OpenHelp);
        } else if key_match(k, KeyAction::Escape) || key_match(k, KeyAction::SwitchTabStatus) {
            self.ctx
                .queue
                .push(InternalEvent::SwitchTab(crate::queue::Tab::Status));
        } else if key_match(k, KeyAction::SwitchTabLog) {
            self.ctx
                .queue
                .push(InternalEvent::SwitchTab(crate::queue::Tab::Log));
        } else if key_match(k, KeyAction::Blame) {
            // not used in log tab
        } else {
            return Ok(EventState::not_consumed());
        }
        Ok(EventState::consumed())
    }
}

impl DrawableComponent for LogComponent {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        let border = if self.focused {
            theme.border_focused
        } else {
            theme.border_unfocused
        };
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(area);

        // ---- left: revision list ----
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .title(TITLE.log);
        let inner = block.inner(chunks[0]);
        f.render_widget(block, chunks[0]);

        if self.pending {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled("Loading...", theme.dim))),
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

        let mut lines: Vec<Line> = Vec::with_capacity(self.entries.len());
        for e in &self.entries {
            lines.push(log_list_line(e, theme));
        }
        let mut scroll = self.scroll.get();
        let view_h = inner.height as usize;
        if view_h > 0 {
            if self.selection < scroll {
                scroll = self.selection;
            } else if self.selection >= scroll + view_h {
                scroll = self.selection - view_h + 1;
            }
        }
        scroll = ui::clamp_scroll(scroll, lines.len(), view_h);
        self.scroll.set(scroll);

        let highlights = vec![(self.selection, Style::default().bg(theme.selection_bg))];
        ui::render_lines(f, inner, &lines, scroll, &highlights);

        // ---- right: details ----
        let block2 = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_unfocused))
            .title(TITLE.log_detail);
        let inner2 = block2.inner(chunks[1]);
        f.render_widget(block2, chunks[1]);

        if let Some(e) = self.entries.get(self.selection) {
            let detail = detail_lines(e, theme);
            let mut dscroll = self.detail_scroll.get();
            dscroll = ui::clamp_scroll(dscroll, detail.len(), inner2.height as usize);
            self.detail_scroll.set(dscroll);
            ui::render_lines(f, inner2, &detail, dscroll, &[]);
        }
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        self.event(ev)
    }
}

fn log_list_line(e: &LogEntry, theme: &Theme) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!("r{}", e.revision), theme.log_revision),
        Span::raw(" "),
        Span::styled(e.author.clone(), theme.log_author),
        Span::raw("  "),
    ];
    // summary: first line of message, capped
    let summary = e.summary();
    let summary = ui::truncate(&summary, 120);
    spans.push(Span::styled(summary, theme.log_message));
    Line::from(spans)
}

fn detail_lines(e: &LogEntry, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    out.push(Line::from(Span::styled(
        format!("r{} | {} | {}", e.revision, e.author, e.date),
        theme.log_revision,
    )));
    out.push(Line::from(""));
    if !e.changed.is_empty() {
        out.push(Line::from(Span::styled("Changed paths:", theme.dim)));
        for (action, path) in &e.changed {
            let style = theme.log_action_style(*action);
            out.push(Line::from(vec![
                Span::styled(format!(" {action} "), style),
                Span::styled(path.clone(), theme.log_message),
            ]));
        }
        out.push(Line::from(""));
    }
    out.push(Line::from(Span::styled("Message:", theme.dim)));
    for line in e.message.lines() {
        out.push(Line::from(Span::styled(
            line.to_string(),
            theme.log_message,
        )));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{ConfirmAction, InternalEvent, NeedsUpdate, Tab};
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn entry(rev: u64, author: &str, msg: &str) -> LogEntry {
        LogEntry {
            revision: rev,
            author: author.to_string(),
            date: "2026-01-01".to_string(),
            line_count: 1,
            changed: vec![('M', "src/main.rs".to_string())],
            message: msg.to_string(),
        }
    }

    fn comp() -> (LogComponent, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut c = LogComponent::new(&ctx);
        c.update(vec![
            entry(3, "alice", "third"),
            entry(2, "bob", "second"),
            entry(1, "alice", "first"),
        ]);
        (c, q)
    }

    #[test]
    fn selection_and_movement() {
        let (mut c, _q) = comp();
        assert_eq!(c.selection_revision(), Some(3));
        c.move_selection(1);
        assert_eq!(c.selection_revision(), Some(2));
        c.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(c.selection_revision(), Some(1));
        c.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap(); // clamp
        assert_eq!(c.selection_revision(), Some(1));
        c.event(&ts::key(crossterm::event::KeyCode::Char('k')))
            .unwrap();
        assert_eq!(c.selection_revision(), Some(2));
        c.event(&ts::key(crossterm::event::KeyCode::End)).unwrap();
        assert_eq!(c.selection_revision(), Some(1));
        c.event(&ts::key(crossterm::event::KeyCode::Home)).unwrap();
        assert_eq!(c.selection_revision(), Some(3));
        // page keys
        c.event(&ts::key(crossterm::event::KeyCode::PageDown))
            .unwrap();
        assert_eq!(c.selection_revision(), Some(1));
        c.event(&ts::key(crossterm::event::KeyCode::PageUp))
            .unwrap();
        assert_eq!(c.selection_revision(), Some(3));
    }

    #[test]
    fn open_revision_diff_and_update_to() {
        let (mut c, q) = comp();
        c.event(&ts::key(crossterm::event::KeyCode::Enter)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestRevisionDiff(3))
        ));
        c.event(&ts::key(crossterm::event::KeyCode::Char('d')))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestRevisionDiff(3))
        ));
        c.event(&ts::key(crossterm::event::KeyCode::Char('o')))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Confirm(ConfirmAction::UpdateToRevision(3)))
        ));
    }

    #[test]
    fn refresh_help_and_tab_switches() {
        let (mut c, q) = comp();
        c.event(&ts::key(crossterm::event::KeyCode::F(5))).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Update(NeedsUpdate::LOG))
        ));
        c.event(&ts::key(crossterm::event::KeyCode::Char('?')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::OpenHelp)));
        c.event(&ts::key(crossterm::event::KeyCode::Esc)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::SwitchTab(Tab::Status))
        ));
        c.event(&ts::key(crossterm::event::KeyCode::Char('1')))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::SwitchTab(Tab::Status))
        ));
        c.event(&ts::key(crossterm::event::KeyCode::Char('2')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::SwitchTab(Tab::Log))));
        // q is not consumed so the app can quit
        assert!(
            !c.event(&ts::key(crossterm::event::KeyCode::Char('q')))
                .unwrap()
                .consumed
        );
    }

    #[test]
    fn draw_list_and_details() {
        let (c, _q) = comp();
        let t = ts::render(120, 20, |f| {
            c.draw(f, Rect::new(0, 0, 120, 20)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("r3"), "{s}");
        assert!(s.contains("alice"), "{s}");
        assert!(s.contains("third"), "{s}");
        assert!(s.contains("Changed paths:"), "{s}");
        assert!(s.contains("src/main.rs"), "{s}");
        assert!(s.contains("Message:"), "{s}");
        assert!(s.contains("Log (svn log)"), "{s}");
    }

    #[test]
    fn draw_loading_and_empty() {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut c = LogComponent::new(&ctx);
        let t1 = ts::render(80, 10, |f| {
            c.draw(f, Rect::new(0, 0, 80, 10)).unwrap();
        });
        assert!(ts::dump(&t1).contains("Loading"));
        c.update(vec![]);
        let t2 = ts::render(80, 10, |f| {
            c.draw(f, Rect::new(0, 0, 80, 10)).unwrap();
        });
        assert!(ts::dump(&t2).contains("No revisions"));
    }

    #[test]
    fn update_clamps_selection() {
        let (mut c, _q) = comp();
        c.event(&ts::key(crossterm::event::KeyCode::End)).unwrap();
        c.update(vec![entry(9, "x", "only")]);
        assert_eq!(c.selection_revision(), Some(9));
    }
}
