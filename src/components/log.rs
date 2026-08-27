//! Log tab: revision list + details (changed paths & message).

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::{ConfirmAction, InternalEvent};
use crate::strings::TITLE;
use crate::svn::models::LogEntry;
use crate::ui::{self, style::Theme};
use crossterm::event::{Event, KeyCode};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use std::cell::Cell;

pub struct LogComponent {
    ctx: Context,
    pub entries: Vec<LogEntry>,
    /// Index into the *filtered* view (see `visible_indices`)
    selection: usize,
    scroll: Cell<usize>,
    detail_scroll: Cell<usize>,
    pub pending: bool,
    pub focused: bool,
    /// Keyword filter over revision/author/message (`/`)
    filter: String,
    filter_active: bool,
    /// Marked revisions for a combined diff (`space`)
    pub marks: std::collections::BTreeSet<u64>,
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
            filter: String::new(),
            filter_active: false,
            marks: std::collections::BTreeSet::new(),
        }
    }

    pub fn update(&mut self, entries: Vec<LogEntry>) {
        self.pending = false;
        self.entries = entries;
        let len = self.visible_indices().len();
        self.selection = self.selection.min(len.saturating_sub(1));
    }

    /// Indices into `entries` that match the current filter.
    fn visible_indices(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.entries.len()).collect();
        }
        let f = self.filter.to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                e.message.to_lowercase().contains(&f)
                    || e.author.to_lowercase().contains(&f)
                    || e.revision.to_string().contains(&f)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn selection_entry(&self) -> Option<&LogEntry> {
        let visible = self.visible_indices();
        let idx = visible.get(self.selection)?;
        self.entries.get(*idx)
    }

    pub fn selection_revision(&self) -> Option<u64> {
        self.selection_entry().map(|e| e.revision)
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.visible_indices().len();
        if len == 0 {
            return;
        }
        self.selection = ui::clamp_index((self.selection as isize + delta).max(0) as usize, len);
        // the detail pane shows the newly selected revision from the top
        self.detail_scroll.set(0);
    }

    fn toggle_mark(&mut self) {
        if let Some(rev) = self.selection_revision()
            && !self.marks.remove(&rev)
        {
            self.marks.insert(rev);
        }
    }

    /// Request a diff: combined when ≥2 revisions are marked, otherwise
    /// the diff of the selected revision.
    fn request_diff(&mut self) {
        if self.marks.len() >= 2 {
            let revs: Vec<u64> = self.marks.iter().copied().collect();
            self.ctx.queue.push(InternalEvent::RequestRangeDiff(revs));
        } else if let Some(rev) = self.selection_revision() {
            self.ctx.queue.push(InternalEvent::RequestRevisionDiff(rev));
        }
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };

        // search input mode captures everything
        if self.filter_active {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.filter_active = false;
                    self.selection = 0;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.selection = 0;
                }
                // ignore control/alt combos (Ctrl+C must not filter 'c')
                KeyCode::Char(c)
                    if !k.modifiers.intersects(
                        crossterm::event::KeyModifiers::CONTROL
                            | crossterm::event::KeyModifiers::ALT,
                    ) =>
                {
                    self.filter.push(c);
                    self.selection = 0;
                }
                _ => {}
            }
            return Ok(EventState::consumed());
        }

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
            self.selection = self.visible_indices().len().saturating_sub(1);
        } else if key_match(k, KeyAction::ToggleMark) {
            self.toggle_mark();
        } else if key_match(k, KeyAction::OpenRevisionDiff) {
            self.request_diff();
        } else if key_match(k, KeyAction::Filter) {
            self.filter_active = true;
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
        } else if key_match(k, KeyAction::Escape) {
            // Esc clears an active filter first, then leaves the tab
            if !self.filter.is_empty() {
                self.filter.clear();
                self.selection = 0;
            } else {
                self.ctx
                    .queue
                    .push(InternalEvent::SwitchTab(crate::queue::Tab::Status));
            }
        } else if key_match(k, KeyAction::SwitchTabStatus) {
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

    /// The `search> ...` input row at the bottom of the list pane.
    fn draw_filter_row(&self, f: &mut Frame, inner: Rect, theme: &Theme) {
        if !self.filter_active || inner.height == 0 {
            return;
        }
        let y = inner.y + inner.height - 1;
        ui::render_line_at(
            f,
            inner.x,
            y,
            inner.width,
            &Line::from(vec![
                Span::styled("search> ", theme.info),
                Span::raw(self.filter.clone()),
            ]),
        );
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
        let mut title = TITLE.log.to_string();
        if !self.filter.is_empty() {
            title.push_str(&format!("  filter: \"{}\"", self.filter));
        }
        if !self.marks.is_empty() {
            title.push_str(&format!("  ({} marked)", self.marks.len()));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .title(title);
        let inner = block.inner(chunks[0]);
        f.render_widget(block, chunks[0]);

        // ---- right: details frame ----
        // always drawn (even while loading/empty): ratatui only diffs
        // cells, so a missing border would leave stale content behind
        let block2 = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_unfocused))
            .title(TITLE.log_detail);
        let inner2 = block2.inner(chunks[1]);
        f.render_widget(block2, chunks[1]);

        if self.pending {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled("Loading...", theme.dim))),
                inner,
            );
            return Ok(());
        }
        let visible = self.visible_indices();
        if visible.is_empty() {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                    crate::strings::MSG.empty_log,
                    theme.dim,
                ))),
                inner,
            );
            self.draw_filter_row(f, inner, theme);
            return Ok(());
        }

        // reserve the last row for the search input while it is active
        let view_h = (inner.height as usize).saturating_sub(usize::from(self.filter_active));
        let mut lines: Vec<Line> = Vec::with_capacity(visible.len().min(view_h.max(1)));
        for &i in &visible {
            lines.push(log_list_line(
                &self.entries[i],
                self.marks.contains(&self.entries[i].revision),
                theme,
            ));
        }
        let mut scroll = self.scroll.get();
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

        self.draw_filter_row(f, inner, theme);

        // ---- right: details content ----
        if let Some(e) = self.selection_entry() {
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

fn log_list_line(e: &LogEntry, marked: bool, theme: &Theme) -> Line<'static> {
    let mut spans = vec![
        Span::styled(if marked { "● " } else { "  " }, theme.status_added),
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
    fn mark_and_request_combined_diff() {
        let (mut c, q) = comp();
        // mark r3 and r1 with space
        c.event(&ts::key(crossterm::event::KeyCode::Char(' ')))
            .unwrap();
        assert!(c.marks.contains(&3));
        c.event(&ts::key(crossterm::event::KeyCode::End)).unwrap();
        c.event(&ts::key(crossterm::event::KeyCode::Char(' ')))
            .unwrap();
        assert!(c.marks.contains(&1));
        // Enter with two marks → combined range diff
        c.event(&ts::key(crossterm::event::KeyCode::Enter)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestRangeDiff(revs)) if revs == vec![1, 3]
        ));
        // unmark again → single-revision diff
        c.event(&ts::key(crossterm::event::KeyCode::Char(' ')))
            .unwrap(); // unmark r1
        c.event(&ts::key(crossterm::event::KeyCode::Home)).unwrap();
        c.event(&ts::key(crossterm::event::KeyCode::Char(' ')))
            .unwrap(); // unmark r3
        assert!(c.marks.is_empty());
        c.event(&ts::key(crossterm::event::KeyCode::Enter)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestRevisionDiff(3))
        ));
    }

    #[test]
    fn search_filters_by_keyword() {
        let (mut c, _q) = comp();
        // '/' enters search mode
        c.event(&ts::key(crossterm::event::KeyCode::Char('/')))
            .unwrap();
        for ch in "second".chars() {
            c.event(&ts::key(crossterm::event::KeyCode::Char(ch)))
                .unwrap();
        }
        assert_eq!(c.selection_revision(), Some(2));
        // Enter keeps the filter and exits input mode
        c.event(&ts::key(crossterm::event::KeyCode::Enter)).unwrap();
        assert_eq!(c.selection_revision(), Some(2));
        // Esc clears the filter
        c.event(&ts::key(crossterm::event::KeyCode::Esc)).unwrap();
        assert_eq!(c.selection_revision(), Some(3));
        // author / revision keywords work too
        c.event(&ts::key(crossterm::event::KeyCode::Char('/')))
            .unwrap();
        c.event(&ts::key(crossterm::event::KeyCode::Char('b')))
            .unwrap();
        assert_eq!(c.selection_revision(), Some(2)); // bob
        c.event(&ts::key(crossterm::event::KeyCode::Backspace))
            .unwrap();
        c.event(&ts::key(crossterm::event::KeyCode::Char('3')))
            .unwrap();
        assert_eq!(c.selection_revision(), Some(3)); // r3
        c.event(&ts::key(crossterm::event::KeyCode::Esc)).unwrap();
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
        let s1 = ts::dump(&t1);
        assert!(s1.contains("Loading"), "{s1}");
        // the detail frame is drawn even while loading (no stale content)
        assert!(s1.contains("Revision details"), "{s1}");
        c.update(vec![]);
        let t2 = ts::render(80, 10, |f| {
            c.draw(f, Rect::new(0, 0, 80, 10)).unwrap();
        });
        let s2 = ts::dump(&t2);
        assert!(s2.contains("No revisions"), "{s2}");
        assert!(s2.contains("Revision details"), "{s2}");
    }

    #[test]
    fn move_selection_resets_detail_scroll() {
        let (mut c, _q) = comp();
        c.detail_scroll.set(5);
        c.move_selection(1);
        assert_eq!(c.detail_scroll.get(), 0);
    }

    #[test]
    fn update_clamps_selection() {
        let (mut c, _q) = comp();
        c.event(&ts::key(crossterm::event::KeyCode::End)).unwrap();
        c.update(vec![entry(9, "x", "only")]);
        assert_eq!(c.selection_revision(), Some(9));
    }
}
