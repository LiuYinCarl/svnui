//! Read-only repository browser tab.
//!
//! This is deliberately a viewer: it lists repository directories at HEAD,
//! previews files (`svn cat`) and shows `svn log` output. It never performs
//! copy/delete/mkdir/switch/update/commit operations.

use super::text_view::TextView;
use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::InternalEvent;
use crate::ui;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::cell::Cell;

/// Status-bar shortcut hints shown while the repository tab is active.
pub const HINTS: &str = "enter/l expand  h collapse  t log  tab preview  F5 reload  esc status";

/// One repository directory/file node.
#[derive(Clone, Debug)]
pub struct RepoEntry {
    pub name: String,
    /// `^/...` caret URL, resolved by svn against the working copy
    pub url: String,
    pub is_dir: bool,
    pub expanded: bool,
    /// A list request is in flight (children may still be empty)
    pub loading: bool,
    pub loaded: bool,
    pub children: Vec<RepoEntry>,
}

impl RepoEntry {
    fn dir(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            is_dir: true,
            expanded: false,
            loading: false,
            loaded: false,
            children: Vec::new(),
        }
    }

    fn file(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            is_dir: false,
            expanded: false,
            loading: false,
            loaded: true,
            children: Vec::new(),
        }
    }
}

/// A flattened visible tree row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleRepo {
    pub depth: usize,
    pub name: String,
    pub url: String,
    pub is_dir: bool,
    pub expanded: bool,
    pub loading: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoPane {
    Tree,
    Preview,
}

pub struct RepoBrowser {
    ctx: Context,
    /// Virtual root (`^/`); always expanded, its children are the top level.
    root: RepoEntry,
    pub selection: usize,
    /// A repository list request is in flight (drives the spinner)
    pub pending: bool,
    pub preview_title: String,
    preview: Vec<String>,
    pub error: Option<String>,
    pub focus: RepoPane,
    pub tv: TextView,
    scroll: Cell<usize>,
}

impl RepoBrowser {
    pub fn new(ctx: &Context) -> Self {
        Self {
            ctx: ctx.clone(),
            root: RepoEntry {
                name: String::new(),
                url: "^/".to_string(),
                is_dir: true,
                expanded: true,
                loading: false,
                loaded: false,
                children: Vec::new(),
            },
            selection: 0,
            pending: false,
            preview_title: "Preview".to_string(),
            preview: Vec::new(),
            error: None,
            focus: RepoPane::Tree,
            tv: TextView::new(),
            scroll: Cell::new(0),
        }
    }

    /// Called when the tab becomes active: load the top level once.
    pub fn activate(&mut self) {
        if !self.root.loaded && !self.root.loading {
            self.request_list("^/");
        }
    }

    /// Reload the top level (F5 when the root is selected).
    pub fn reload_root(&mut self) {
        self.root.loaded = false;
        self.request_list("^/");
    }

    fn request_list(&mut self, url: &str) {
        let Some(node) = find_mut(&mut self.root, url) else {
            return;
        };
        if node.loading {
            return;
        }
        node.loading = true;
        node.loaded = false;
        self.pending = true;
        self.ctx
            .queue
            .push(InternalEvent::RepoLoad(url.to_string()));
    }

    pub fn apply_list(&mut self, url: &str, result: Result<Vec<(String, bool)>, String>) {
        self.pending = false;
        let Some(node) = find_mut(&mut self.root, url) else {
            return;
        };
        node.loading = false;
        match result {
            Ok(entries) => {
                let mut children: Vec<RepoEntry> = entries
                    .into_iter()
                    .map(|(name, is_dir)| {
                        let child_url = child_url(&node.url, &name);
                        if is_dir {
                            RepoEntry::dir(name, child_url)
                        } else {
                            RepoEntry::file(name, child_url)
                        }
                    })
                    .collect();
                children.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                node.children = children;
                node.loaded = true;
                node.expanded = true;
                self.error = None;
                self.clamp_selection();
            }
            Err(e) => {
                self.error = Some(format!("svn list {url}: {e}"));
            }
        }
    }

    pub fn apply_file(&mut self, url: &str, result: Result<String, String>) {
        if self.selected().is_none_or(|s| s.url != url) {
            return;
        }
        match result {
            Ok(content) => {
                self.preview = content.lines().map(str::to_string).collect();
                self.preview_title = format!("Preview: {url}");
                self.tv.reset();
                self.error = None;
            }
            Err(e) => {
                self.preview.clear();
                self.preview_title = format!("Preview: {url}");
                self.error = Some(e);
            }
        }
    }

    pub fn visible(&self) -> Vec<VisibleRepo> {
        let mut out = Vec::new();
        walk(&self.root, 0, &mut out);
        out
    }

    pub fn selected(&self) -> Option<VisibleRepo> {
        self.visible().into_iter().nth(self.selection)
    }

    fn clamp_selection(&mut self) {
        let len = self.visible().len();
        if len == 0 {
            self.selection = 0;
        } else if self.selection >= len {
            self.selection = len - 1;
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            return;
        }
        self.selection = ui::clamp_index((self.selection as isize + delta).max(0) as usize, len);
    }

    fn toggle_selected(&mut self) {
        let Some(sel) = self.selected() else {
            return;
        };
        if sel.is_dir {
            let Some(node) = find_mut(&mut self.root, &sel.url) else {
                return;
            };
            node.expanded = !node.expanded;
            if node.expanded && !node.loaded && !node.loading {
                self.request_list(&sel.url);
            }
            self.clamp_selection();
        } else {
            self.ctx.queue.push(InternalEvent::RepoPreview(sel.url));
        }
    }

    fn collapse_selected(&mut self) {
        let Some(sel) = self.selected() else {
            return;
        };
        if sel.is_dir
            && let Some(node) = find_mut(&mut self.root, &sel.url)
            && node.expanded
        {
            node.expanded = false;
            self.clamp_selection();
        }
    }

    fn request_log(&mut self) {
        if let Some(sel) = self.selected() {
            self.ctx.queue.push(InternalEvent::RepoLog(sel.url));
        }
    }

    fn reload_selected(&mut self) {
        let Some(sel) = self.selected() else {
            self.reload_root();
            return;
        };
        if sel.is_dir {
            if let Some(node) = find_mut(&mut self.root, &sel.url) {
                node.loaded = false;
                node.expanded = true;
            }
            self.request_list(&sel.url);
        } else {
            self.ctx.queue.push(InternalEvent::RepoPreview(sel.url));
        }
    }

    fn tree_lines(&self) -> (Vec<Line<'static>>, Vec<VisibleRepo>) {
        let theme = &self.ctx.theme;
        let visible = self.visible();
        let mut lines = Vec::with_capacity(visible.len());
        for row in &visible {
            let mut spans = vec![Span::raw("  ".repeat(row.depth))];
            if row.is_dir {
                spans.push(Span::styled(
                    if row.loading {
                        "… "
                    } else if row.expanded {
                        "▼ "
                    } else {
                        "▶ "
                    },
                    theme.dim,
                ));
                spans.push(Span::styled(
                    format!("{}/", row.name),
                    theme.text.add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(row.name.clone(), theme.text));
            }
            lines.push(Line::from(spans));
        }
        (lines, visible)
    }
}

fn child_url(parent: &str, name: &str) -> String {
    if parent.ends_with('/') {
        format!("{parent}{name}")
    } else {
        format!("{parent}/{name}")
    }
}

fn find_mut<'a>(node: &'a mut RepoEntry, url: &str) -> Option<&'a mut RepoEntry> {
    if node.url == url {
        return Some(node);
    }
    for child in &mut node.children {
        if let Some(found) = find_mut(child, url) {
            return Some(found);
        }
    }
    None
}

fn walk(node: &RepoEntry, depth: usize, out: &mut Vec<VisibleRepo>) {
    for child in &node.children {
        out.push(VisibleRepo {
            depth,
            name: child.name.clone(),
            url: child.url.clone(),
            is_dir: child.is_dir,
            expanded: child.expanded,
            loading: child.loading,
        });
        if child.is_dir && child.expanded {
            walk(child, depth + 1, out);
        }
    }
}

impl DrawableComponent for RepoBrowser {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        let (lines, _visible) = self.tree_lines();
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);

        let tree_border = if self.focus == RepoPane::Tree {
            theme.border_focused
        } else {
            theme.border_unfocused
        };
        let tree_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(tree_border))
            .title("Repository (read-only)");
        let tree_inner = tree_block.inner(chunks[0]);
        f.render_widget(tree_block, chunks[0]);
        if self.root.loading && lines.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled("Loading repository...", theme.dim))),
                tree_inner,
            );
        } else if lines.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    self.error.as_deref().unwrap_or("Repository is empty"),
                    theme.dim,
                ))),
                tree_inner,
            );
        } else {
            let scroll = ui::scroll_follow(
                self.selection,
                self.scroll.get(),
                lines.len(),
                tree_inner.height as usize,
            );
            self.scroll.set(scroll);
            let highlights = [(self.selection, Style::default().bg(theme.selection_bg))];
            ui::render_lines(f, tree_inner, &lines, scroll, &highlights);
        }

        let preview_border = if self.focus == RepoPane::Preview {
            theme.border_focused
        } else {
            theme.border_unfocused
        };
        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(preview_border))
            .title(self.preview_title.clone());
        let preview_inner = preview_block.inner(chunks[1]);
        f.render_widget(preview_block, chunks[1]);
        if let Some(err) = &self.error {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    err.clone(),
                    theme.status_conflicted,
                )))
                .wrap(ratatui::widgets::Wrap { trim: false }),
                preview_inner,
            );
        } else if self.preview.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "Enter a file to preview · t shows its log",
                    theme.dim,
                ))),
                preview_inner,
            );
        } else {
            let lines: Vec<Line> = self
                .preview
                .iter()
                .map(|l| Line::from(Span::styled(l.clone(), theme.text)))
                .collect();
            let scroll = ui::clamp_scroll(
                self.tv.scroll.get(),
                lines.len(),
                preview_inner.height as usize,
            );
            self.tv.scroll.set(scroll);
            ui::render_lines_h(f, preview_inner, &lines, scroll, &[], self.tv.hscroll.get());
        }
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::FocusNext) || key_match(k, KeyAction::FocusPrev) {
            self.focus = match self.focus {
                RepoPane::Tree => RepoPane::Preview,
                RepoPane::Preview => RepoPane::Tree,
            };
            return Ok(EventState::consumed());
        }
        if self.focus == RepoPane::Preview {
            let len = self.preview.len();
            let state = self.tv.scroll_event(ev, len);
            if state.consumed {
                return Ok(state);
            }
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
            self.selection = self.visible().len().saturating_sub(1);
        } else if key_match(k, KeyAction::MoveLeft) {
            self.collapse_selected();
        } else if key_match(k, KeyAction::MoveRight) || key_match(k, KeyAction::Enter) {
            self.toggle_selected();
        } else if key_match(k, KeyAction::FileHistory) {
            self.request_log();
        } else if key_match(k, KeyAction::Refresh) {
            self.reload_selected();
        } else if key_match(k, KeyAction::Escape) {
            self.ctx
                .queue
                .push(InternalEvent::SwitchTab(crate::queue::Tab::Status));
        } else {
            return Ok(EventState::not_consumed());
        }
        Ok(EventState::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{Queue, Tab};
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

    fn comp() -> (RepoBrowser, Queue) {
        let (c, q) = ctx();
        (RepoBrowser::new(&c), q)
    }

    #[test]
    fn activate_requests_root_once() {
        let (mut c, q) = comp();
        c.activate();
        assert!(matches!(q.pop(), Some(InternalEvent::RepoLoad(url)) if url == "^/"));
        assert!(c.pending);
        c.activate();
        assert!(q.pop().is_none(), "root already loading");
    }

    #[test]
    fn apply_list_builds_sorted_tree_and_expands_dir() {
        let (mut c, q) = comp();
        c.activate();
        q.pop();
        c.apply_list(
            "^/",
            Ok(vec![
                ("trunk".into(), true),
                ("readme.md".into(), false),
                ("branches".into(), true),
            ]),
        );
        assert!(!c.pending);
        let visible = c.visible();
        assert_eq!(visible[0].name, "branches");
        assert_eq!(visible[1].name, "trunk");
        assert_eq!(visible[2].name, "readme.md");
        // expand trunk: requests its children
        c.selection = 1;
        c.event(&ts::key(crossterm::event::KeyCode::Enter)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::RepoLoad(url)) if url == "^/trunk"));
        c.apply_list("^/trunk", Ok(vec![("a.txt".into(), false)]));
        let visible = c.visible();
        assert!(visible.iter().any(|e| e.name == "a.txt" && e.depth == 1));
    }

    #[test]
    fn file_preview_only_applies_to_current_selection() {
        let (mut c, q) = comp();
        c.activate();
        q.pop();
        c.apply_list("^/", Ok(vec![("a.txt".into(), false)]));
        c.apply_file("a.txt", Ok("hello\n".into()));
        assert!(c.preview.is_empty(), "url must use ^/ prefix");
        c.apply_file("^/a.txt", Ok("hello\n".into()));
        assert_eq!(c.preview, vec!["hello"]);
        assert!(c.preview_title.contains("^/a.txt"));
    }

    #[test]
    fn log_and_preview_keys_queue_read_only_requests() {
        let (mut c, q) = comp();
        c.activate();
        q.pop();
        c.apply_list("^/", Ok(vec![("a.txt".into(), false)]));
        c.event(&ts::key(crossterm::event::KeyCode::Char('t')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::RepoLog(url)) if url == "^/a.txt"));
        c.event(&ts::key(crossterm::event::KeyCode::Enter)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::RepoPreview(url)) if url == "^/a.txt"));
    }

    #[test]
    fn draw_renders_tree_and_preview() {
        let (mut c, q) = comp();
        c.activate();
        q.pop();
        c.apply_list(
            "^/",
            Ok(vec![("branches".into(), true), ("a.txt".into(), false)]),
        );
        c.selection = 1;
        c.apply_file("^/a.txt", Ok("content\n".into()));
        let t = ts::render(100, 20, |f| {
            c.draw(f, Rect::new(0, 0, 100, 20)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("Repository (read-only)"), "{s}");
        assert!(s.contains("branches/"), "{s}");
        assert!(s.contains("content"), "{s}");
    }

    #[test]
    fn escape_returns_to_status_tab() {
        let (mut c, q) = comp();
        c.event(&ts::key(crossterm::event::KeyCode::Esc)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::SwitchTab(Tab::Status))
        ));
    }
}
