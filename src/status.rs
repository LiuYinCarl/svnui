//! The Status tab: file tree + diff pane + commit bar.

use super::components::{
    Context, DrawableComponent, EventState, commit::CommitComponent, diff_view::DiffView,
    status_tree::StatusTreeComponent,
};
use crate::keys::{KeyAction, key_match};

use crate::svn::models::StatusEntry;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneFocus {
    Tree,
    Diff,
    Commit,
}

pub struct StatusTab {
    pub tree: StatusTreeComponent,
    pub diff: DiffView,
    pub commit: CommitComponent,
    pub focus: PaneFocus,
    pub last_diff_requested: Option<String>,
}

impl StatusTab {
    pub fn new(ctx: &Context) -> Self {
        let mut diff = DiffView::new("Diff");
        diff.set_hint(
            "Diff".to_string(),
            "Select a file to view its diff".to_string(),
        );
        Self {
            tree: StatusTreeComponent::new(ctx),
            diff,
            commit: CommitComponent::new(ctx),
            focus: PaneFocus::Tree,
            last_diff_requested: None,
        }
    }

    pub fn set_focus(&mut self, focus: PaneFocus) {
        self.focus = focus;
        self.tree.set_focused(focus == PaneFocus::Tree);
        self.diff.focused = focus == PaneFocus::Diff;
        self.commit.focused = focus == PaneFocus::Commit;
    }

    pub fn cycle_focus(&mut self, forward: bool) {
        let next = match self.focus {
            PaneFocus::Tree => {
                if forward {
                    PaneFocus::Diff
                } else {
                    PaneFocus::Commit
                }
            }
            PaneFocus::Diff => {
                if forward {
                    PaneFocus::Commit
                } else {
                    PaneFocus::Tree
                }
            }
            PaneFocus::Commit => {
                if forward {
                    PaneFocus::Tree
                } else {
                    PaneFocus::Diff
                }
            }
        };
        self.set_focus(next);
    }

    // ----- data updates -----

    pub fn update_status(&mut self, entries: Vec<StatusEntry>) {
        self.tree.update(entries);
        // force the diff to reload after status changes
        self.last_diff_requested = None;
        self.update_commit_hint();
        if self.tree.is_empty() && self.tree.selection_entry().is_none() {
            self.diff
                .set_hint("Diff".to_string(), "Working copy is clean".to_string());
        }
    }

    pub fn apply_diff(&mut self, path: &str, content: &str) {
        if self.tree.selection_path().as_deref() == Some(path) {
            self.diff.set_content(path.to_string(), content);
        }
    }

    /// Request a diff for the currently selected file if it changed.
    /// Returns true when a request was issued (caller bumps pending counter).
    pub fn maybe_request_diff(&mut self) -> Option<String> {
        let sel = self.tree.selection_path();
        let is_file = self.tree.selection_entry().is_some();
        if !is_file {
            if let Some(path) = &sel {
                let reason = if self.tree.is_empty() {
                    "Working copy is clean".to_string()
                } else {
                    format!("{path} is a directory — select a file")
                };
                self.diff.set_hint("Diff".to_string(), reason);
            }
            self.last_diff_requested = sel;
            return None;
        }
        let path = sel?;
        if self.last_diff_requested.as_deref() == Some(path.as_str()) {
            return None;
        }
        self.last_diff_requested = Some(path.clone());
        self.diff.set_loading(path.clone());
        Some(path)
    }

    pub fn update_commit_hint(&mut self) {
        let staged = self.tree.staged_count();
        self.commit.hint = if staged > 0 {
            format!("{staged} file(s) staged")
        } else {
            "no files staged — commit all changes".to_string()
        };
    }

    pub fn set_staged(&mut self, paths: &[String]) {
        self.tree.set_staged(paths);
        self.update_commit_hint();
    }

    pub fn unset_staged(&mut self, paths: &[String]) {
        self.tree.unset_staged(paths);
        self.update_commit_hint();
    }

    pub fn clear_staged(&mut self) {
        self.tree.clear_staged();
        self.update_commit_hint();
    }

    // ----- events -----

    pub fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        // Focused pane first
        let consumed = match self.focus {
            PaneFocus::Tree => self.tree.event(ev)?.consumed,
            PaneFocus::Diff => self.diff.event(ev).consumed || self.tree.event(ev)?.consumed,
            PaneFocus::Commit => self.commit.event(ev)?.consumed || self.tree.event(ev)?.consumed,
        };
        // After commit unfocuses itself (Esc), return focus to the tree
        if self.focus == PaneFocus::Commit && !self.commit.focused {
            self.set_focus(PaneFocus::Tree);
        }
        Ok(EventState { consumed })
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.commit.ctx.theme;
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(3)])
            .split(area);
        let content = chunks[0];
        let horiz = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(content);
        self.tree.draw(f, horiz[0])?;
        crate::components::diff_view::draw_diff_block(f, horiz[1], &self.diff, theme);
        self.commit.draw(f, chunks[1])?;
        Ok(())
    }

    pub fn handle_global_key(&mut self, ev: &Event) -> bool {
        let Event::Key(k) = ev else {
            return false;
        };
        if key_match(k, KeyAction::FocusNext) || key_match(k, KeyAction::FocusPrev) {
            self.cycle_focus(key_match(k, KeyAction::FocusNext));
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svn::models::StatusEntry;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn entry(status: char, path: &str) -> StatusEntry {
        StatusEntry {
            status,
            props_status: ' ',
            tree_conflict: ' ',
            path: path.to_string(),
            is_dir: std::path::Path::new(path).is_dir(),
        }
    }

    fn tab() -> (StatusTab, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        (StatusTab::new(&ctx), q)
    }

    #[test]
    fn focus_cycling() {
        let (mut t, _q) = tab();
        assert_eq!(t.focus, PaneFocus::Tree);
        t.cycle_focus(true);
        assert_eq!(t.focus, PaneFocus::Diff);
        t.cycle_focus(true);
        assert_eq!(t.focus, PaneFocus::Commit);
        t.cycle_focus(true);
        assert_eq!(t.focus, PaneFocus::Tree);
        t.cycle_focus(false);
        assert_eq!(t.focus, PaneFocus::Commit);
        t.cycle_focus(false);
        assert_eq!(t.focus, PaneFocus::Diff);
        // component focus flags stay in sync
        t.set_focus(PaneFocus::Tree);
        assert!(t.tree.focused);
        assert!(!t.commit.focused);
        t.set_focus(PaneFocus::Commit);
        assert!(t.commit.focused);
        assert!(!t.tree.focused);
    }

    #[test]
    fn update_status_sets_hints() {
        let (mut t, _q) = tab();
        t.update_status(vec![entry('M', "a.txt"), entry('?', "b.txt")]);
        assert_eq!(t.tree.staged_count(), 0);
        assert!(t.commit.hint.contains("no files staged"));
        // staged hint
        t.tree.set_staged(&["a.txt".to_string()]);
        t.update_commit_hint();
        assert!(t.commit.hint.contains("1 file(s) staged"));
        // clean working copy hint
        t.update_status(vec![]);
        assert_eq!(
            t.diff.empty_reason.as_deref(),
            Some("Working copy is clean")
        );
    }

    #[test]
    fn maybe_request_diff_flow() {
        let (mut t, _q) = tab();
        t.update_status(vec![entry('M', "a.txt"), entry('M', "b.txt")]);
        // first selection (a.txt) requests a diff
        assert_eq!(t.maybe_request_diff().as_deref(), Some("a.txt"));
        // same file → no duplicate request
        assert_eq!(t.maybe_request_diff(), None);
        // new status: selection stays on a.txt (a file) → new request
        t.update_status(vec![entry('M', "src/main.rs"), entry('M', "a.txt")]);
        assert_eq!(t.maybe_request_diff().as_deref(), Some("a.txt"));
        // moving to the src dir → hint, no request
        t.tree
            .event(&crossterm::event::Event::Key(
                crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('k')),
            ))
            .unwrap();
        assert_eq!(t.maybe_request_diff(), None);
        assert_eq!(
            t.diff.empty_reason.as_deref(),
            Some("src is a directory — select a file")
        );
        // status change forces a new request
        t.update_status(vec![entry('M', "a.txt")]);
        assert_eq!(t.maybe_request_diff().as_deref(), Some("a.txt"));
    }

    #[test]
    fn apply_diff_only_for_current_selection() {
        let (mut t, _q) = tab();
        t.update_status(vec![entry('M', "a.txt")]);
        // stale diff for a different path is ignored (hint stays)
        t.apply_diff("b.txt", "Index: b\n");
        assert!(t.diff.empty_reason.is_some());
        t.apply_diff("a.txt", "Index: a.txt\n@@ -1 +1 @@\n-old\n+new\n");
        assert!(t.diff.empty_reason.is_none());
        assert_eq!(t.diff.parsed.lines.len(), 4);
    }

    #[test]
    fn event_routes_to_focused_pane() {
        let (mut t, q) = tab();
        t.update_status(vec![entry('M', "a.txt")]);
        // tree focused: 'j' consumed
        assert!(
            t.event(&ts::key(crossterm::event::KeyCode::Char('j')))
                .unwrap()
                .consumed
        );
        // diff focused: scroll keys handled by diff, 'j' not consumed by diff
        // but falls through to the tree
        t.set_focus(PaneFocus::Diff);
        let _ = t
            .event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        // commit focused: typing consumed by commit
        t.set_focus(PaneFocus::Commit);
        let _ = t
            .event(&ts::key(crossterm::event::KeyCode::Char('x')))
            .unwrap();
        assert_eq!(t.commit.text, "x");
        // Esc in commit unfocuses and returns focus to tree
        let _ = t.event(&ts::key(crossterm::event::KeyCode::Esc)).unwrap();
        assert!(!t.commit.focused);
        assert_eq!(t.focus, PaneFocus::Tree);
        // tree pushes queue events (e.g. commit via 'c')
        t.event(&ts::key(crossterm::event::KeyCode::Char('c')))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(crate::queue::InternalEvent::OpenCommit)
        ));
    }

    #[test]
    fn handle_global_key_cycles_focus() {
        let (mut t, _q) = tab();
        assert!(t.handle_global_key(&ts::key(crossterm::event::KeyCode::Tab)));
        assert_eq!(t.focus, PaneFocus::Diff);
        let shift_tab = crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::BackTab,
            crossterm::event::KeyModifiers::SHIFT,
        ));
        assert!(t.handle_global_key(&shift_tab));
        assert_eq!(t.focus, PaneFocus::Tree);
        assert!(!t.handle_global_key(&ts::key(crossterm::event::KeyCode::Char('z'))));
    }

    #[test]
    fn staged_helpers() {
        let (mut t, _q) = tab();
        t.set_staged(&["a.txt".to_string()]);
        assert_eq!(t.tree.staged_count(), 1);
        t.unset_staged(&["a.txt".to_string()]);
        assert_eq!(t.tree.staged_count(), 0);
        t.set_staged(&["a.txt".to_string()]);
        t.clear_staged();
        assert_eq!(t.tree.staged_count(), 0);
    }

    #[test]
    fn draw_layout() {
        let (mut t, _q) = tab();
        t.update_status(vec![entry('M', "a.txt")]);
        t.tree
            .event(&crossterm::event::Event::Key(
                crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('j')),
            ))
            .unwrap();
        t.apply_diff("a.txt", "Index: a.txt\n===\n@@ -1 +1 @@\n-old\n+new\n");
        let terminal = ts::render(120, 30, |f| {
            t.draw(f, Rect::new(0, 0, 120, 30)).unwrap();
        });
        let s = ts::dump(&terminal);
        assert!(s.contains("a.txt"), "{s}");
        assert!(s.contains("Commit message"), "{s}");
        assert!(s.contains("old"), "{s}");
        assert!(s.contains("new"), "{s}");
    }
}
