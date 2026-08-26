//! The application state: tabs, popup stack, async operation dispatch.
//! Modeled on gitui's `App` + `Gitui`.

use crate::components::{Context, DrawableComponent, log::LogComponent};
use crate::keys::{KeyAction, key_match};
use crate::popups::Popup;
use crate::queue::{ConfirmAction, InternalEvent, NeedsUpdate, Queue, Tab};
use crate::status::StatusTab;
use crate::strings::MSG;
use crate::svn::models::BlameLine;
use crate::svn::{AsyncSvnNotification, Svn};
use crate::ui;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::cell::Cell;
use std::path::PathBuf;

/// A diff request that will turn into a fullscreen popup when loaded.
#[derive(Clone, Debug)]
enum PendingFullscreen {
    File(String),
    Revision(u64),
}

pub struct App {
    pub svn: Svn,
    pub queue: Queue,
    pub ctx: Context,
    pub status: StatusTab,
    pub log: LogComponent,
    pub active_tab: Tab,
    pub popups: Vec<Popup>,
    /// Number of outstanding async operations (drives the spinner)
    pub pending: usize,
    pub quitting: bool,
    pub fatal_error: Option<String>,
    pub spinner_frame: Cell<usize>,
    pending_fullscreen: Option<PendingFullscreen>,
}

impl App {
    pub fn new(_cwd: PathBuf, svn: Svn, ctx: Context) -> Self {
        let status = StatusTab::new(&ctx);
        let log = LogComponent::new(&ctx);
        Self {
            svn,
            queue: ctx.queue.clone(),
            status,
            log,
            ctx,
            active_tab: Tab::Status,
            popups: Vec::new(),
            pending: 0,
            quitting: false,
            fatal_error: None,
            spinner_frame: Cell::new(0),
            pending_fullscreen: None,
        }
    }

    // ----- popup helpers -----

    fn push_popup(&mut self, popup: Popup) {
        self.popups.push(popup);
    }

    fn pop_popup(&mut self) {
        self.popups.pop();
    }

    fn show_error(&mut self, msg: String) {
        let ctx = self.ctx.clone();
        self.push_popup(Popup::msg(&ctx, msg, true));
    }

    fn show_info(&mut self, msg: String) {
        let ctx = self.ctx.clone();
        self.push_popup(Popup::msg(&ctx, msg, false));
    }

    fn show_confirm(&mut self, message: String, action: ConfirmAction) {
        let ctx = self.ctx.clone();
        self.push_popup(Popup::confirm(&ctx, message, action));
    }

    fn show_output(&mut self, title: String, content: &str) {
        let ctx = self.ctx.clone();
        self.push_popup(Popup::output(&ctx, title, content));
    }

    fn show_diff_popup(&mut self, title: String, content: &str) {
        let ctx = self.ctx.clone();
        self.push_popup(Popup::diff(&ctx, title, content));
    }

    // ----- startup -----

    pub fn start(&mut self) {
        self.svn.check_info();
        self.pending += 1;
    }

    // ----- event dispatch -----

    pub fn handle_input(&mut self, ev: &Event) -> Result<(), String> {
        // Popups get the first chance; while one is open, tab interaction
        // is blocked entirely.
        if let Some(popup) = self.popups.last_mut() {
            popup.event(ev)?;
            // process popup actions (ClosePopup, Confirmed, ...)
            self.handle_queue_events();
            // Fatal errors: quit once the user dismissed the message
            if self.fatal_error.is_some() && self.popups.is_empty() {
                self.quitting = true;
            }
            return Ok(());
        }
        if self.fatal_error.is_some() {
            self.quitting = true;
            return Ok(());
        }

        // Active tab
        let consumed = match self.active_tab {
            Tab::Status => self.status.event(ev)?.consumed || self.status.handle_global_key(ev),
            Tab::Log => self.log.event(ev)?.consumed,
        };
        if consumed {
            return Ok(());
        }

        // App-level keys
        let Event::Key(k) = ev else {
            return Ok(());
        };
        if key_match(k, KeyAction::Quit) {
            self.quitting = true;
        } else if key_match(k, KeyAction::Help) {
            let ctx = self.ctx.clone();
            self.push_popup(Popup::help(&ctx));
        } else if key_match(k, KeyAction::FocusNext) {
            if self.active_tab == Tab::Status {
                self.status.cycle_focus(true);
            } else {
                self.active_tab = Tab::Status;
            }
        } else if key_match(k, KeyAction::FocusPrev) {
            if self.active_tab == Tab::Status {
                self.status.cycle_focus(false);
            } else {
                self.active_tab = Tab::Status;
            }
        } else if key_match(k, KeyAction::SwitchTabStatus) {
            self.active_tab = Tab::Status;
        } else if key_match(k, KeyAction::SwitchTabLog) {
            self.active_tab = Tab::Log;
        }
        Ok(())
    }

    /// Called after every input event and after status updates.
    pub fn maybe_request_diff(&mut self) {
        if self.active_tab != Tab::Status {
            return;
        }
        if let Some(path) = self.status.maybe_request_diff() {
            self.svn.diff(&path);
            self.pending += 1;
        }
    }

    pub fn tick(&mut self) {
        self.spinner_frame
            .set(self.spinner_frame.get().saturating_add(1));
    }

    // ----- queue events (pushed by components) -----

    pub fn handle_queue_events(&mut self) {
        for ev in self.queue.drain() {
            self.handle_internal(ev);
        }
    }

    fn handle_internal(&mut self, ev: InternalEvent) {
        match ev {
            InternalEvent::Update(flags) => {
                if flags.contains(NeedsUpdate::STATUS) || flags.contains(NeedsUpdate::ALL) {
                    self.svn.status();
                    self.pending += 1;
                }
                if flags.contains(NeedsUpdate::LOG) || flags.contains(NeedsUpdate::ALL) {
                    self.svn.log(50);
                    self.pending += 1;
                }
            }
            InternalEvent::ShowInfoMsg(msg) => self.show_info(msg),
            InternalEvent::OpenHelp => {
                let ctx = self.ctx.clone();
                self.push_popup(Popup::help(&ctx));
            }
            InternalEvent::ClosePopup => self.pop_popup(),
            InternalEvent::OpenCommit => {
                self.status.set_focus(crate::status::PaneFocus::Commit);
                self.status.commit.focus();
            }
            InternalEvent::Confirm(action) => {
                let message = match &action {
                    ConfirmAction::Commit { message, paths } => {
                        if message.trim().is_empty() {
                            self.show_error("Commit message is empty".to_string());
                            return;
                        }
                        let what = if paths.is_empty() {
                            let staged = self.status.tree.staged_count();
                            if staged > 0 {
                                format!("{} ({staged} files)", MSG.commit_staged)
                            } else {
                                MSG.commit_all.to_string()
                            }
                        } else {
                            format!("{} ({} files)", MSG.commit_staged, paths.len())
                        };
                        format!("{what}\n\n\"{}\"", message.trim())
                    }
                    ConfirmAction::Revert(paths) => {
                        format!("{} ({})", MSG.revert_confirm, paths.join(", "))
                    }
                    ConfirmAction::Update => MSG.update_confirm.to_string(),
                    ConfirmAction::Resolve(path) => {
                        format!("{} ({path})", MSG.resolve_confirm)
                    }
                    ConfirmAction::UpdateToRevision(rev) => {
                        format!("{} (r{rev})", MSG.update_to_rev_confirm)
                    }
                };
                self.show_confirm(message, action);
            }
            InternalEvent::Confirmed(action) => {
                self.perform_confirmed(action);
            }
            InternalEvent::SwitchTab(tab) => {
                self.active_tab = tab;
            }
            InternalEvent::RefreshStatus => {
                self.svn.status();
                self.pending += 1;
            }
            InternalEvent::AddFiles(paths) => {
                if !paths.is_empty() {
                    self.svn.add(&paths);
                    self.pending += 1;
                }
            }
            InternalEvent::RequestFileDiff => {
                if let (Some(path), true) = (
                    self.status.tree.selection_path(),
                    self.status.tree.selection_entry().is_some(),
                ) {
                    self.pending_fullscreen = Some(PendingFullscreen::File(path.clone()));
                    self.svn.diff(&path);
                    self.pending += 1;
                }
            }
            InternalEvent::RequestBlame => {
                if let Some(e) = self.status.tree.selection_entry() {
                    if !e.is_dir && e.status == '?' {
                        self.show_error("svn blame: file is not under version control".to_string());
                    } else if !e.is_dir {
                        let path = e.path.clone();
                        let ctx = self.ctx.clone();
                        self.push_popup(Popup::blame(&ctx, &path));
                        self.svn.blame(&path);
                        self.pending += 1;
                    }
                }
            }
            InternalEvent::RequestRevisionDiff(rev) => {
                self.pending_fullscreen = Some(PendingFullscreen::Revision(rev));
                self.svn.revision_diff(rev);
                self.pending += 1;
            }
        }
    }

    fn perform_confirmed(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Commit { message, mut paths } => {
                if paths.is_empty() {
                    paths = self.status.tree.staged.iter().cloned().collect();
                }
                self.svn.commit(&message, &paths);
                self.pending += 1;
            }
            ConfirmAction::Revert(paths) => {
                self.svn.revert(&paths);
                self.pending += 1;
            }
            ConfirmAction::Update => {
                self.svn.update();
                self.pending += 1;
            }
            ConfirmAction::Resolve(path) => {
                self.svn.resolve(&path);
                self.pending += 1;
            }
            ConfirmAction::UpdateToRevision(rev) => {
                self.svn.update_to_revision(rev);
                self.pending += 1;
            }
        }
    }

    // ----- async results -----

    pub fn handle_async(&mut self, notif: AsyncSvnNotification) {
        self.pending = self.pending.saturating_sub(1);
        match notif {
            AsyncSvnNotification::Info(result) => match result {
                Ok(()) => {
                    self.svn.status();
                    self.svn.log(50);
                    self.pending += 2;
                }
                Err(e) => {
                    let msg = format!("{}\n{e}", MSG.no_working_copy);
                    self.fatal_error = Some(msg.clone());
                    self.show_error(msg);
                }
            },
            AsyncSvnNotification::Status(result) => match result {
                Ok(entries) => {
                    self.status.update_status(entries);
                    self.maybe_request_diff();
                }
                Err(e) => self.show_error(format!("svn status: {e}")),
            },
            AsyncSvnNotification::Diff { path, result } => match result {
                Ok(content) => {
                    self.status.apply_diff(&path, &content);
                    if let Some(PendingFullscreen::File(p)) = &self.pending_fullscreen
                        && *p == path
                    {
                        self.pending_fullscreen = None;
                        self.show_diff_popup(path.clone(), &content);
                    }
                }
                Err(e) => self.show_error(format!("svn diff {path}: {e}")),
            },
            AsyncSvnNotification::Log(result) => match result {
                Ok(entries) => self.log.update(entries),
                Err(e) => self.show_error(format!("svn log: {e}")),
            },
            AsyncSvnNotification::RevisionDiff { revision, result } => match result {
                Ok(content) => {
                    if let Some(PendingFullscreen::Revision(r)) = &self.pending_fullscreen
                        && *r == revision
                    {
                        self.pending_fullscreen = None;
                        self.show_diff_popup(format!("Diff r{revision}"), &content);
                    }
                }
                Err(e) => self.show_error(format!("svn diff -c {revision}: {e}")),
            },
            AsyncSvnNotification::Blame { path, result } => match result {
                Ok(lines) => self.update_blame_popup(&path, lines),
                Err(e) => self.show_error(format!("svn blame {path}: {e}")),
            },
            AsyncSvnNotification::Update(result) => match result {
                Ok(out) => {
                    self.show_output("svn update".to_string(), &out);
                    self.refresh_after_op();
                }
                Err(e) => {
                    self.show_error(format!("svn update: {e}"));
                    self.refresh_after_op();
                }
            },
            AsyncSvnNotification::UpdateToRevision(result) => match result {
                Ok(out) => {
                    self.show_output("svn update -r".to_string(), &out);
                    self.refresh_after_op();
                }
                Err(e) => {
                    self.show_error(format!("svn update: {e}"));
                    self.refresh_after_op();
                }
            },
            AsyncSvnNotification::Commit(result) => match result {
                Ok(out) => {
                    self.status.clear_staged();
                    self.status.commit.text.clear();
                    self.status.commit.unfocus();
                    self.show_output("svn commit".to_string(), &out);
                    self.refresh_after_op();
                }
                Err(e) => self.show_error(format!("svn commit: {e}")),
            },
            AsyncSvnNotification::Add(result) => match result {
                Ok(paths) => {
                    self.status.set_staged(&paths);
                    self.show_info(MSG.add_done.to_string());
                    self.refresh_after_op();
                }
                Err(e) => {
                    self.show_error(format!("svn add: {e}"));
                }
            },
            AsyncSvnNotification::Revert(result) => match result {
                Ok(paths) => {
                    self.status.unset_staged(&paths);
                    self.show_info(MSG.revert_done.to_string());
                    self.refresh_after_op();
                }
                Err(e) => self.show_error(format!("svn revert: {e}")),
            },
            AsyncSvnNotification::Resolve(result) => match result {
                Ok(path) => {
                    self.show_info(format!("{}: {path}", MSG.resolve_done));
                    self.refresh_after_op();
                }
                Err(e) => self.show_error(format!("svn resolve: {e}")),
            },
        }
    }

    fn update_blame_popup(&mut self, path: &str, lines: Vec<BlameLine>) {
        for popup in self.popups.iter_mut().rev() {
            if let Popup::Blame(blame) = popup
                && blame.path == path
            {
                blame.update(lines);
                return;
            }
        }
    }

    fn refresh_after_op(&mut self) {
        self.svn.status();
        self.pending += 1;
        if self.active_tab == Tab::Log {
            self.svn.log(50);
            self.pending += 1;
        }
    }

    // ----- drawing -----

    pub fn draw(&mut self, f: &mut Frame) -> Result<(), String> {
        let area = f.area();
        let status_bar_h = 1u16;
        let main = Rect::new(
            area.x,
            area.y,
            area.width,
            area.height.saturating_sub(status_bar_h),
        );

        match self.active_tab {
            Tab::Status => self.status.draw(f, main)?,
            Tab::Log => self.log.draw(f, main)?,
        }

        // status bar
        self.draw_status_bar(
            f,
            Rect::new(area.x, area.y + main.height, area.width, status_bar_h),
        );

        // popups
        for popup in &self.popups {
            let rect = popup.rect(area);
            popup.draw(f, rect)?;
        }

        // cursor for the commit input
        // only shown when the commit input is focused; otherwise the
        // cursor stays hidden automatically.
        if self.active_tab == Tab::Status && self.status.commit.focused {
            let (x, y) = self.status.commit.cursor_pos.get();
            f.set_cursor_position((x, y));
        }
        Ok(())
    }

    fn draw_status_bar(&self, f: &mut Frame, area: Rect) {
        let theme = &self.ctx.theme;
        let mut spans = Vec::new();
        if self.pending > 0 {
            let frame = ui::spinner_frame(self.spinner_frame.get());
            spans.push(Span::styled(format!("{frame} "), theme.info));
            spans.push(Span::styled(format!("{} op", self.pending), theme.info));
            spans.push(Span::raw(" "));
        }
        let staged = self.status.tree.staged_count();
        if staged > 0 {
            spans.push(Span::styled(
                format!("staged {staged} "),
                theme.status_added,
            ));
        }
        spans.push(Span::styled(
            "? help  c commit  u update  F5 refresh  q quit  [1]status [2]log",
            theme.dim,
        ));
        let line = Line::from(spans);
        f.buffer_mut().set_line(area.x, area.y, &line, area.width);
    }
}

#[allow(dead_code)]
fn _unused(_: Style) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Context;
    use crate::queue::{ConfirmAction, InternalEvent, NeedsUpdate, Tab};
    use crate::svn::models::{BlameLine, LogEntry, StatusEntry};
    use crate::test_support::{self, TestRepo};
    use crate::ui::style::Theme;
    use crossbeam_channel::unbounded;
    use crossterm::event::KeyCode;
    use ratatui::backend::TestBackend;
    use std::time::Duration;

    fn entry(status: char, path: &str) -> StatusEntry {
        StatusEntry {
            status,
            props_status: ' ',
            tree_conflict: ' ',
            path: path.to_string(),
            is_dir: std::path::Path::new(path).is_dir(),
        }
    }

    fn log_entry(rev: u64, msg: &str) -> LogEntry {
        LogEntry {
            revision: rev,
            author: "alice".into(),
            date: "2026-01-01".into(),
            line_count: 1,
            changed: vec![('M', "src/main.rs".into())],
            message: msg.into(),
        }
    }

    fn app_with(repo: &TestRepo) -> (App, crossbeam_channel::Receiver<AsyncSvnNotification>) {
        let (tx, rx) = unbounded();
        let queue = Queue::new();
        let ctx = Context {
            queue: queue.clone(),
            theme: Theme::default(),
        };
        let svn = Svn::new(repo.wc.clone(), tx);
        let app = App::new(repo.wc.clone(), svn, ctx);
        (app, rx)
    }

    fn recv<T>(rx: &crossbeam_channel::Receiver<T>) -> T {
        rx.recv_timeout(Duration::from_secs(15))
            .expect("timeout waiting for svn result")
    }

    #[test]
    fn start_checks_info_and_loads() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, rx) = app_with(&repo);
        app.start();
        // wait for the working-copy check
        assert!(matches!(recv(&rx), AsyncSvnNotification::Info(Ok(()))));
        // processing it triggers status + log fetches (order not guaranteed)
        app.handle_async(AsyncSvnNotification::Info(Ok(())));
        let first = recv(&rx);
        let second = recv(&rx);
        assert!(matches!(
            first,
            AsyncSvnNotification::Status(_) | AsyncSvnNotification::Log(_)
        ));
        assert!(matches!(
            second,
            AsyncSvnNotification::Status(_) | AsyncSvnNotification::Log(_)
        ));
        assert!(matches!(
            (&first, &second),
            (
                AsyncSvnNotification::Status(_),
                AsyncSvnNotification::Log(_)
            ) | (
                AsyncSvnNotification::Log(_),
                AsyncSvnNotification::Status(_)
            )
        ));
        assert!(app.pending > 0);
    }

    #[test]
    fn fatal_info_error_sets_flag_and_msg() {
        let Some(repo) = TestRepo::new() else { return };
        let dir = std::env::temp_dir().join(format!("svnui-fatal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = unbounded();
        let queue = Queue::new();
        let ctx = Context {
            queue,
            theme: Theme::default(),
        };
        let mut app = App::new(dir.clone(), Svn::new(dir.clone(), tx), ctx);
        app.handle_async(AsyncSvnNotification::Info(Err("not a wc".into())));
        assert!(app.fatal_error.is_some());
        assert_eq!(app.popups.len(), 1);
        let _ = rx;
        let _ = std::fs::remove_dir_all(&dir);
        let _ = repo;
    }

    #[test]
    fn status_log_diff_updates_components() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, rx) = app_with(&repo);
        app.handle_async(AsyncSvnNotification::Status(Ok(vec![
            entry('M', "a.txt"),
            entry('?', "b.txt"),
        ])));
        assert_eq!(app.status.tree.selection_path().as_deref(), Some("a.txt"));
        // diff gets requested (async) and applies when it arrives
        assert!(matches!(recv(&rx), AsyncSvnNotification::Diff { .. }));
        app.handle_async(AsyncSvnNotification::Diff {
            path: "a.txt".into(),
            result: Ok("Index: a.txt\n@@ -1 +1 @@\n-old\n+new\n".into()),
        });
        assert!(!app.status.diff.pending);

        app.handle_async(AsyncSvnNotification::Log(Ok(vec![
            log_entry(3, "three"),
            log_entry(2, "two"),
        ])));
        assert_eq!(app.log.selection_revision(), Some(3));

        // error paths
        app.handle_async(AsyncSvnNotification::Status(Err("boom".into())));
        assert!(matches!(app.popups.last(), Some(Popup::Msg(_))));
        app.handle_async(AsyncSvnNotification::Log(Err("boom".into())));
        app.handle_async(AsyncSvnNotification::Diff {
            path: "a.txt".into(),
            result: Err("boom".into()),
        });
    }

    #[test]
    fn revision_diff_and_blame_flow() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, rx) = app_with(&repo);
        // load a status so the tree has a selected file (top-level so the
        // dir-collapsed tree selects it directly)
        app.handle_async(AsyncSvnNotification::Status(Ok(vec![entry(
            'M', "main.rs",
        )])));
        // that triggers a diff request for the selection; consume it
        assert!(matches!(recv(&rx), AsyncSvnNotification::Diff { path, .. } if path == "main.rs"));
        // request revision diff through the queue
        app.queue.push(InternalEvent::RequestRevisionDiff(1));
        app.handle_queue_events();
        assert!(matches!(
            recv(&rx),
            AsyncSvnNotification::RevisionDiff { revision: 1, .. }
        ));
        app.handle_async(AsyncSvnNotification::RevisionDiff {
            revision: 1,
            result: Ok("Index: x\n===\n@@ -1 +1 @@\n-a\n+b\n".into()),
        });
        assert!(matches!(app.popups.last(), Some(Popup::Diff(_))));

        // blame request → popup + async load
        app.popups.clear();
        app.queue.push(InternalEvent::RequestBlame);
        app.handle_queue_events();
        assert!(matches!(app.popups.last(), Some(Popup::Blame(_))));
        assert!(matches!(recv(&rx), AsyncSvnNotification::Blame { .. }));
        app.handle_async(AsyncSvnNotification::Blame {
            path: "main.rs".into(),
            result: Ok(vec![BlameLine {
                revision: Some(1),
                author: "kenshin".into(),
                content: "fn main() {}".into(),
            }]),
        });

        // blame popup got the data
        let has_data = app.popups.iter().any(|p| match p {
            Popup::Blame(b) => !b.pending && b.lines.len() == 1,
            _ => false,
        });
        assert!(has_data);
    }

    #[test]
    fn file_diff_fullscreen_matches_selection() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, rx) = app_with(&repo);
        app.handle_async(AsyncSvnNotification::Status(Ok(vec![entry('M', "a.txt")])));
        let _ = recv(&rx); // diff request for a.txt
        app.queue.push(InternalEvent::RequestFileDiff);
        app.handle_queue_events();
        assert!(matches!(recv(&rx), AsyncSvnNotification::Diff { .. }));
        app.handle_async(AsyncSvnNotification::Diff {
            path: "a.txt".into(),
            result: Ok("Index: a.txt\n===\n@@ -1 +1 @@\n-a\n+b\n".into()),
        });
        assert!(matches!(app.popups.last(), Some(Popup::Diff(_))));
    }

    #[test]
    fn update_commit_add_revert_resolve_async_flows() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, rx) = app_with(&repo);

        // update ok → output popup + refresh; then resolve result arrives
        app.handle_async(AsyncSvnNotification::Update(Ok(
            "Updating '.':\nAt revision 1.\n".into(),
        )));
        assert!(matches!(app.popups.last(), Some(Popup::Output(_))));
        assert!(matches!(
            recv(&rx),
            AsyncSvnNotification::Status(_) | AsyncSvnNotification::Log(_)
        ));
        app.handle_async(AsyncSvnNotification::Update(Err("network down".into())));

        // commit ok → clears staged, output popup, refresh
        app.status.set_staged(&["a.txt".into()]);
        app.handle_async(AsyncSvnNotification::Commit(Ok(
            "Committed revision 9.\n".into()
        )));
        assert_eq!(app.status.tree.staged_count(), 0);
        assert!(matches!(app.popups.last(), Some(Popup::Output(_))));
        assert!(matches!(recv(&rx), AsyncSvnNotification::Status(_)));

        // commit error → error popup, staged stays
        app.status.set_staged(&["a.txt".into()]);
        app.handle_async(AsyncSvnNotification::Commit(Err("E170000: stale".into())));
        assert_eq!(app.status.tree.staged_count(), 1);
        assert!(matches!(app.popups.last(), Some(Popup::Msg(_))));

        // add ok → staged + refresh; add err → error
        app.handle_async(AsyncSvnNotification::Add(Ok(vec!["b.txt".into()])));
        assert!(app.status.tree.staged.contains("b.txt"));
        assert!(matches!(recv(&rx), AsyncSvnNotification::Status(_)));
        app.handle_async(AsyncSvnNotification::Add(Err("E155010".into())));

        // revert ok → unstaged + refresh
        app.status.set_staged(&["b.txt".into()]);
        app.handle_async(AsyncSvnNotification::Revert(Ok(vec!["b.txt".into()])));
        assert!(!app.status.tree.staged.contains("b.txt"));
        assert!(matches!(recv(&rx), AsyncSvnNotification::Status(_)));
        app.handle_async(AsyncSvnNotification::Revert(Err("boom".into())));

        // resolve ok / err
        app.handle_async(AsyncSvnNotification::Resolve(Ok("c.txt".into())));
        assert!(matches!(recv(&rx), AsyncSvnNotification::Status(_)));
        app.handle_async(AsyncSvnNotification::Resolve(Err("boom".into())));

        // update to revision ok / err
        app.handle_async(AsyncSvnNotification::UpdateToRevision(Ok(
            "Updated to r1.".into()
        )));
        assert!(matches!(recv(&rx), AsyncSvnNotification::Status(_)));
        app.handle_async(AsyncSvnNotification::UpdateToRevision(Err("boom".into())));
    }

    #[test]
    fn confirmed_actions_run_real_svn() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, rx) = app_with(&repo);

        // commit all (no staged)
        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 7\n");
        app.perform_confirmed(ConfirmAction::Commit {
            message: "bump".into(),
            paths: vec![],
        });
        assert!(matches!(recv(&rx), AsyncSvnNotification::Commit(_)));

        // revert
        test_support::write_file(&repo.wc.join("docs/readme.md"), "# v2\n");
        app.perform_confirmed(ConfirmAction::Revert(vec!["docs/readme.md".into()]));
        assert!(matches!(recv(&rx), AsyncSvnNotification::Revert(_)));

        // update + update to revision + resolve
        app.perform_confirmed(ConfirmAction::Update);
        assert!(matches!(recv(&rx), AsyncSvnNotification::Update(_)));
        app.perform_confirmed(ConfirmAction::UpdateToRevision(1));
        assert!(matches!(
            recv(&rx),
            AsyncSvnNotification::UpdateToRevision(_)
        ));
        app.perform_confirmed(ConfirmAction::Resolve("Cargo.toml".into()));
        assert!(matches!(recv(&rx), AsyncSvnNotification::Resolve(_)));

        // commit with explicit paths
        app.perform_confirmed(ConfirmAction::Commit {
            message: "docs change".into(),
            paths: vec!["docs/readme.md".into()],
        });
        assert!(matches!(recv(&rx), AsyncSvnNotification::Commit(_)));
    }

    #[test]
    fn confirm_event_opens_popup_and_validates_message() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, _rx) = app_with(&repo);
        app.queue
            .push(InternalEvent::Confirm(ConfirmAction::Commit {
                message: "  ".into(),
                paths: vec![],
            }));
        app.handle_queue_events();
        assert!(matches!(app.popups.last(), Some(Popup::Msg(_))));
        assert!(app.popups.len() == 1);

        app.popups.clear();
        app.queue
            .push(InternalEvent::Confirm(ConfirmAction::Commit {
                message: "good message".into(),
                paths: vec![],
            }));
        app.handle_queue_events();
        assert!(matches!(app.popups.last(), Some(Popup::Confirm(_))));
        app.popups.clear();

        // revert / update / resolve / update-to-rev confirmations
        for action in [
            ConfirmAction::Revert(vec!["a.txt".into()]),
            ConfirmAction::Update,
            ConfirmAction::Resolve("a.txt".into()),
            ConfirmAction::UpdateToRevision(4),
        ] {
            app.queue.push(InternalEvent::Confirm(action.clone()));
            app.handle_queue_events();
            assert!(
                matches!(app.popups.last(), Some(Popup::Confirm(_))),
                "popup for {action:?}"
            );
            app.popups.clear();
        }
    }

    #[test]
    fn queue_events_update_and_refresh() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, rx) = app_with(&repo);
        app.queue.push(InternalEvent::Update(NeedsUpdate::ALL));
        app.handle_queue_events();
        let m1 = recv(&rx);
        let m2 = recv(&rx);
        assert!(matches!(
            m1,
            AsyncSvnNotification::Status(_) | AsyncSvnNotification::Log(_)
        ));
        assert!(matches!(
            m2,
            AsyncSvnNotification::Status(_) | AsyncSvnNotification::Log(_)
        ));

        app.queue.push(InternalEvent::Update(NeedsUpdate::LOG));
        app.handle_queue_events();
        assert!(matches!(recv(&rx), AsyncSvnNotification::Log(_)));

        app.queue.push(InternalEvent::RefreshStatus);
        app.handle_queue_events();
        assert!(matches!(recv(&rx), AsyncSvnNotification::Status(_)));

        app.queue
            .push(InternalEvent::AddFiles(vec!["x.txt".into()]));
        app.handle_queue_events();
        assert!(matches!(recv(&rx), AsyncSvnNotification::Add(_)));

        // OpenCommit focuses the commit pane
        app.queue.push(InternalEvent::OpenCommit);
        app.handle_queue_events();
        assert!(app.status.commit.focused);
        assert_eq!(app.status.focus, crate::status::PaneFocus::Commit);

        // tab switching
        app.queue.push(InternalEvent::SwitchTab(Tab::Log));
        app.handle_queue_events();
        assert_eq!(app.active_tab, Tab::Log);

        // help / close popups / info
        app.queue.push(InternalEvent::OpenHelp);
        app.handle_queue_events();
        assert!(matches!(app.popups.last(), Some(Popup::Help(_))));
        app.queue.push(InternalEvent::ClosePopup);
        app.handle_queue_events();
        assert!(app.popups.is_empty());
        app.queue.push(InternalEvent::ShowInfoMsg("hi".into()));
        app.handle_queue_events();
        assert!(matches!(app.popups.last(), Some(Popup::Msg(_))));
    }

    #[test]
    fn handle_input_routing() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, _rx) = app_with(&repo);
        app.handle_async(AsyncSvnNotification::Status(Ok(vec![entry('M', "a.txt")])));

        // q quits
        app.handle_input(&ts_key(KeyCode::Char('q'))).unwrap();
        assert!(app.quitting);
        app.quitting = false;

        // '?' is consumed by the tree and queues OpenHelp
        app.handle_input(&ts_key(KeyCode::Char('?'))).unwrap();
        app.handle_queue_events();
        assert!(!app.popups.is_empty());
        app.handle_input(&ts_key(KeyCode::Char('q'))).unwrap();
        assert!(!app.quitting, "popup blocks quit");

        // Esc closes the help popup
        app.handle_input(&ts_key(KeyCode::Esc)).unwrap();
        app.handle_queue_events();
        assert!(app.popups.is_empty());

        // Tab cycles focus in status tab
        app.handle_input(&ts_key(KeyCode::Tab)).unwrap();
        assert_eq!(app.status.focus, crate::status::PaneFocus::Diff);
        app.handle_input(&ts_key(KeyCode::Tab)).unwrap();
        assert_eq!(app.status.focus, crate::status::PaneFocus::Commit);

        // Esc returns focus to the tree, then number keys switch tabs
        // (queued by the tree component)
        app.handle_input(&ts_key(KeyCode::Esc)).unwrap();
        assert_eq!(app.status.focus, crate::status::PaneFocus::Tree);
        app.handle_input(&ts_key(KeyCode::Char('2'))).unwrap();
        app.handle_queue_events();
        assert_eq!(app.active_tab, Tab::Log);
        app.handle_input(&ts_key(KeyCode::Char('1'))).unwrap();
        app.handle_queue_events();
        assert_eq!(app.active_tab, Tab::Status);
    }

    #[test]
    fn fatal_error_quits_after_popup_dismissed() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, _rx) = app_with(&repo);
        app.fatal_error = Some("fatal".into());
        // popup still open: keys go to popup; after it closes we quit
        app.push_popup(Popup::msg(&app.ctx.clone(), "fatal".into(), true));
        app.handle_input(&ts_key(KeyCode::Char('x'))).unwrap();
        app.handle_queue_events();
        assert!(app.popups.is_empty());
        assert!(app.quitting);
    }

    #[test]
    fn tick_advances_spinner() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, _rx) = app_with(&repo);
        let before = app.spinner_frame.get();
        app.tick();
        assert_eq!(app.spinner_frame.get(), before + 1);
    }

    #[test]
    fn maybe_request_diff_only_in_status_tab() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, rx) = app_with(&repo);
        app.active_tab = Tab::Log;
        app.maybe_request_diff();
        assert!(app.pending == 0);
        app.active_tab = Tab::Status;
        app.handle_async(AsyncSvnNotification::Status(Ok(vec![entry('M', "a.txt")])));
        let _ = recv(&rx); // diff request
    }

    #[test]
    fn draw_status_log_and_popups() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, _rx) = app_with(&repo);
        app.handle_async(AsyncSvnNotification::Status(Ok(vec![entry('M', "a.txt")])));
        app.handle_async(AsyncSvnNotification::Log(Ok(vec![log_entry(3, "three")])));

        // status tab
        let backend = TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| app.draw(f).unwrap()).unwrap();
        let s = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(s.contains("a.txt"), "{s}");
        assert!(s.contains("Commit message"), "{s}");
        assert!(s.contains("? help"), "{s}");

        // log tab
        app.active_tab = Tab::Log;
        terminal.draw(|f| app.draw(f).unwrap()).unwrap();
        let s2 = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(s2.contains("three"), "{s2}");
        assert!(s2.contains("Log (svn log)"), "{s2}");

        // popup drawn on top
        app.active_tab = Tab::Status;
        app.push_popup(Popup::msg(&app.ctx.clone(), "popup text".into(), true));
        terminal.draw(|f| app.draw(f).unwrap()).unwrap();
        let s3 = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(s3.contains("popup text"), "{s3}");
    }

    #[test]
    fn refresh_after_op_also_refreshes_log_in_log_tab() {
        let Some(repo) = TestRepo::new() else { return };
        let (mut app, rx) = app_with(&repo);
        app.active_tab = Tab::Log;
        app.refresh_after_op();
        let m1 = recv(&rx);
        let m2 = recv(&rx);
        assert!(matches!(
            m1,
            AsyncSvnNotification::Status(_) | AsyncSvnNotification::Log(_)
        ));
        assert!(matches!(
            m2,
            AsyncSvnNotification::Status(_) | AsyncSvnNotification::Log(_)
        ));
    }

    fn ts_key(code: crossterm::event::KeyCode) -> crossterm::event::Event {
        crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    #[allow(dead_code)]
    fn _unused(_: &mut App) {}
}
