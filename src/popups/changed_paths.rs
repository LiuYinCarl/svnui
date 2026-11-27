//! Read-only list of the paths changed by one revision.

use super::super::components::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::InternalEvent;
use crate::ui;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use std::cell::Cell;

pub struct ChangedPathsPopup {
    ctx: Context,
    pub revision: u64,
    /// `(action char, path)` exactly as parsed from `svn log -v`; a copy
    /// line still carries its "(from /path:rev)" suffix for display.
    pub paths: Vec<(char, String)>,
    selection: usize,
    scroll: Cell<usize>,
}

/// Strip the ` (from /path:rev)` suffix svn appends to copy/move targets.
pub fn changed_path_target(raw: &str) -> &str {
    raw.split_once(" (from ")
        .map(|(path, _)| path)
        .unwrap_or(raw)
        .trim()
}

impl ChangedPathsPopup {
    pub fn new(ctx: &Context, revision: u64, paths: Vec<(char, String)>) -> Self {
        Self {
            ctx: ctx.clone(),
            revision,
            paths,
            selection: 0,
            scroll: Cell::new(0),
        }
    }

    fn clamp(&mut self) {
        if self.paths.is_empty() {
            self.selection = 0;
        } else {
            self.selection = self.selection.min(self.paths.len() - 1);
        }
    }
}

impl DrawableComponent for ChangedPathsPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(format!("Changed paths in r{}", self.revision));
        let inner = block.inner(area);
        f.render_widget(block, area);

        if self.paths.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "No changed paths in this revision",
                    theme.dim,
                ))),
                inner,
            );
            return Ok(());
        }

        let lines: Vec<Line> = self
            .paths
            .iter()
            .map(|(action, path)| {
                Line::from(vec![
                    Span::styled(format!("{action} "), theme.status_style(*action)),
                    Span::styled(path.clone(), theme.text),
                ])
            })
            .collect();
        let scroll = ui::scroll_follow(
            self.selection,
            self.scroll.get(),
            lines.len(),
            inner.height as usize,
        );
        self.scroll.set(scroll);
        let highlights = [(
            self.selection,
            Style::default()
                .bg(theme.selection_bg)
                .add_modifier(Modifier::BOLD),
        )];
        ui::render_lines(f, inner, &lines, scroll, &highlights);
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::MoveUp) {
            self.selection = self.selection.saturating_sub(1);
        } else if key_match(k, KeyAction::MoveDown) {
            self.selection = (self.selection + 1).min(self.paths.len().saturating_sub(1));
        } else if key_match(k, KeyAction::PageUp) {
            self.selection = self.selection.saturating_sub(20);
        } else if key_match(k, KeyAction::PageDown) {
            self.selection = (self.selection + 20).min(self.paths.len().saturating_sub(1));
        } else if key_match(k, KeyAction::Home) {
            self.selection = 0;
        } else if key_match(k, KeyAction::End) {
            self.selection = self.paths.len().saturating_sub(1);
        } else if key_match(k, KeyAction::Enter) {
            let path = self
                .paths
                .get(self.selection)
                .map(|(_, p)| changed_path_target(p).to_string());
            if let Some(path) = path {
                self.ctx.queue.push(InternalEvent::ClosePopup);
                self.ctx.queue.push(InternalEvent::RequestRevisionDiff(
                    self.revision,
                    Some(path),
                ));
            }
            return Ok(EventState::consumed());
        } else if key_match(k, KeyAction::ClosePopup) || key_match(k, KeyAction::Quit) {
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        } else {
            return Ok(EventState::not_consumed());
        }
        self.clamp();
        Ok(EventState::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Queue;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn ctx() -> (Context, Queue) {
        let q = Queue::new();
        (
            Context {
                queue: q.clone(),
                theme: Theme::default(),
            },
            q,
        )
    }

    #[test]
    fn strips_copy_suffix_from_target() {
        assert_eq!(changed_path_target("branches/feat"), "branches/feat");
        assert_eq!(
            changed_path_target("branches/feat (from trunk:42)"),
            "branches/feat"
        );
        assert_eq!(
            changed_path_target("dir/with space (from ^/trunk:7)"),
            "dir/with space"
        );
    }

    #[test]
    fn enter_requests_path_limited_revision_diff() {
        let (c, q) = ctx();
        let mut p = ChangedPathsPopup::new(
            &c,
            7,
            vec![
                ('M', "a.txt".into()),
                ('A', "branches/x (from ^/trunk:3)".into()),
            ],
        );
        p.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        p.event(&ts::key(crossterm::event::KeyCode::Enter)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestRevisionDiff(7, Some(path)))
                if path == "branches/x"
        ));
    }

    #[test]
    fn esc_closes_without_requesting() {
        let (c, q) = ctx();
        let mut p = ChangedPathsPopup::new(&c, 1, vec![('M', "a".into())]);
        p.event(&ts::key(crossterm::event::KeyCode::Esc)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(q.pop().is_none());
    }
}
