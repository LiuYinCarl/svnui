//! Status file tree: shows `svn status` entries grouped by directory,
//! supports staging (commit set), filtering, and selection-driven diff.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::{ConfirmAction, InternalEvent};
use crate::strings::{self, TITLE};
use crate::svn::models::{StatusEntry, TreeItem, TreeItemKind};
use crate::ui;
use crossterm::event::{Event, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use std::collections::HashSet;

/// Internal tree node while building.
#[derive(Clone, Debug)]
struct Node {
    name: String,
    path: String,
    is_dir: bool,
    /// Index into `entries` if this path has its own status entry
    entry: Option<usize>,
    children: Vec<Node>,
}

pub struct StatusTreeComponent {
    ctx: Context,
    entries: Vec<StatusEntry>,
    visible: Vec<TreeItem>,
    expanded: HashSet<String>,
    selection: usize,
    scroll: std::cell::Cell<usize>,
    /// Paths included in the commit set (staged)
    pub staged: HashSet<String>,
    filter: String,
    filter_active: bool,
    pub pending: bool,
    pub focused: bool,
}

impl StatusTreeComponent {
    pub fn new(ctx: &Context) -> Self {
        Self {
            ctx: ctx.clone(),
            entries: Vec::new(),
            visible: Vec::new(),
            expanded: HashSet::new(),
            selection: 0,
            scroll: std::cell::Cell::new(0),
            staged: HashSet::new(),
            filter: String::new(),
            filter_active: false,
            pending: true,
            focused: true,
        }
    }

    // ----- public API used by the app -----

    pub fn update(&mut self, entries: Vec<StatusEntry>) {
        self.pending = false;
        self.entries = entries;
        self.rebuild_visible();
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    pub fn selection_path(&self) -> Option<String> {
        self.visible.get(self.selection).map(|t| t.path.clone())
    }

    pub fn selection_entry(&self) -> Option<&StatusEntry> {
        let item = self.visible.get(self.selection)?;
        match &item.kind {
            TreeItemKind::File { entry } => self.entries.get(*entry),
            TreeItemKind::Dir { .. } => None,
        }
    }

    /// Paths affected by staging the item under the cursor.
    pub fn paths_at_selection(&self) -> Vec<String> {
        let Some(item) = self.visible.get(self.selection) else {
            return Vec::new();
        };
        match &item.kind {
            TreeItemKind::File { .. } => vec![item.path.clone()],
            TreeItemKind::Dir { .. } => self.files_under(&item.path),
        }
    }

    /// Toggle staging for the item under the cursor. Returns the paths that
    /// were newly staged (so the app can run `svn add` for unversioned ones)
    /// and the paths that were unstaged.
    pub fn toggle_stage_at_selection(&mut self) -> (Vec<String>, Vec<String>) {
        let paths = self.paths_at_selection();
        self.toggle_stage_paths(&paths)
    }

    pub fn toggle_stage_paths(&mut self, paths: &[String]) -> (Vec<String>, Vec<String>) {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        for p in paths {
            if self.staged.contains(p) {
                self.staged.remove(p);
                removed.push(p.clone());
            } else {
                self.staged.insert(p.clone());
                added.push(p.clone());
            }
        }
        (added, removed)
    }

    pub fn set_staged(&mut self, paths: &[String]) {
        for p in paths {
            self.staged.insert(p.clone());
        }
    }

    pub fn unset_staged(&mut self, paths: &[String]) {
        for p in paths {
            self.staged.remove(p);
        }
    }

    pub fn clear_staged(&mut self) {
        self.staged.clear();
    }

    pub fn staged_count(&self) -> usize {
        self.staged.len()
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    // ----- internals -----

    fn files_under(&self, dir_path: &str) -> Vec<String> {
        let prefix = format!("{dir_path}/");
        self.entries
            .iter()
            .filter(|e| !e.is_dir && e.path.starts_with(&prefix))
            .map(|e| e.path.clone())
            .collect()
    }

    fn rebuild_visible(&mut self) {
        let prev = self.selection_path();
        let mut visible = Vec::new();
        if self.filter.is_empty() {
            let nodes = build_tree(&self.entries);
            flatten(&nodes, &self.expanded, 0, &mut visible);
        } else {
            let f = self.filter.to_lowercase();
            for (i, e) in self.entries.iter().enumerate() {
                if e.is_dir {
                    continue;
                }
                if e.path.to_lowercase().contains(&f) {
                    visible.push(TreeItem {
                        depth: 0,
                        path: e.path.clone(),
                        name: e.path.clone(),
                        kind: TreeItemKind::File { entry: i },
                    });
                }
            }
        }
        self.visible = visible;
        self.selection = self
            .visible
            .iter()
            .position(|t| Some(t.path.as_str()) == prev.as_deref())
            .unwrap_or(0);
        if self.selection >= self.visible.len() && !self.visible.is_empty() {
            self.selection = self.visible.len() - 1;
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.visible.len();
        if len == 0 {
            return;
        }
        self.selection = ui::clamp_index((self.selection as isize + delta).max(0) as usize, len);
    }

    fn collapse_or_expand(&mut self, expand: bool) {
        let Some(item) = self.visible.get(self.selection) else {
            return;
        };
        if let TreeItemKind::Dir { .. } = item.kind {
            if expand {
                self.expanded.insert(item.path.clone());
            } else {
                self.expanded.remove(&item.path);
            }
            self.rebuild_visible();
        }
    }

    // ----- event handling -----

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };

        // Filter input mode captures everything
        if self.filter_active {
            match k.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.filter_active = false;
                    self.rebuild_visible();
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.rebuild_visible();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.rebuild_visible();
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
            self.selection = self.visible.len().saturating_sub(1);
        } else if key_match(k, KeyAction::MoveLeft) {
            self.collapse_or_expand(false);
        } else if key_match(k, KeyAction::MoveRight) {
            self.collapse_or_expand(true);
        } else if key_match(k, KeyAction::Enter) {
            if let Some(item) = self.visible.get(self.selection) {
                match item.kind {
                    TreeItemKind::Dir { .. } => {
                        self.collapse_or_expand(true);
                    }
                    TreeItemKind::File { .. } => {
                        self.ctx.queue.push(InternalEvent::RequestFileDiff);
                    }
                }
            }
        } else if key_match(k, KeyAction::ToggleStage) {
            let (added, _) = self.toggle_stage_at_selection();
            if !added.is_empty() {
                let needs_add: Vec<String> = added
                    .iter()
                    .filter(|p| self.entry_for_path(p).is_some_and(|e| e.status == '?'))
                    .cloned()
                    .collect();
                if !needs_add.is_empty() {
                    self.ctx.queue.push(InternalEvent::AddFiles(needs_add));
                }
            }
        } else if key_match(k, KeyAction::AddFiles) {
            let paths = self.paths_at_selection();
            if !paths.is_empty() {
                let unversioned: Vec<String> = paths
                    .iter()
                    .filter(|p| {
                        self.entry_for_path(p).is_none()
                            || self.entry_for_path(p).is_some_and(|e| e.status == '?')
                    })
                    .cloned()
                    .collect();
                if !unversioned.is_empty() {
                    self.ctx.queue.push(InternalEvent::AddFiles(unversioned));
                }
            }
        } else if key_match(k, KeyAction::RevertFiles) {
            let paths: Vec<String> = self
                .paths_at_selection()
                .into_iter()
                .filter(|p| self.entry_for_path(p).is_some_and(|e| e.status != '?'))
                .collect();
            if !paths.is_empty() {
                self.ctx
                    .queue
                    .push(InternalEvent::Confirm(ConfirmAction::Revert(paths)));
            } else {
                self.ctx.queue.push(InternalEvent::ShowInfoMsg(
                    "No versioned files to revert".to_string(),
                ));
            }
        } else if key_match(k, KeyAction::ResolveConflict) {
            if let Some(e) = self.selection_entry()
                && e.is_conflicted()
            {
                self.ctx
                    .queue
                    .push(InternalEvent::Confirm(ConfirmAction::Resolve(
                        e.path.clone(),
                    )));
            }
        } else if key_match(k, KeyAction::Commit) {
            self.ctx.queue.push(InternalEvent::OpenCommit);
        } else if key_match(k, KeyAction::UpdateWc) {
            self.ctx
                .queue
                .push(InternalEvent::Confirm(ConfirmAction::Update));
        } else if key_match(k, KeyAction::Filter) {
            self.filter_active = true;
        } else if key_match(k, KeyAction::DiffFull) {
            if self.selection_entry().is_some() {
                self.ctx.queue.push(InternalEvent::RequestFileDiff);
            }
        } else if key_match(k, KeyAction::Blame) {
            if let Some(e) = self.selection_entry()
                && !e.is_dir
            {
                self.ctx.queue.push(InternalEvent::RequestBlame);
            }
        } else if key_match(k, KeyAction::Refresh) {
            self.ctx.queue.push(InternalEvent::RefreshStatus);
        } else if key_match(k, KeyAction::Help) {
            self.ctx.queue.push(InternalEvent::OpenHelp);
        } else if key_match(k, KeyAction::SwitchTabLog) {
            self.ctx
                .queue
                .push(InternalEvent::SwitchTab(crate::queue::Tab::Log));
        } else if key_match(k, KeyAction::SwitchTabStatus) {
            self.ctx
                .queue
                .push(InternalEvent::SwitchTab(crate::queue::Tab::Status));
        } else if key_match(k, KeyAction::OpenRevisionDiff) {
            // 'd'/'Enter' handled above as DiffFull; nothing here
        } else {
            return Ok(EventState::not_consumed());
        }
        Ok(EventState::consumed())
    }

    fn entry_for_path(&self, path: &str) -> Option<&StatusEntry> {
        self.entries.iter().find(|e| e.path == path)
    }
}

impl DrawableComponent for StatusTreeComponent {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        let border = if self.focused {
            theme.border_focused
        } else {
            theme.border_unfocused
        };
        let mut title = TITLE.status.to_string();
        if self.pending {
            title.push_str("  (loading)");
        } else if !self.filter.is_empty() {
            title.push_str(&format!("  filter: \"{}\"", self.filter));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border))
            .title(title);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let content_height = inner.height as usize;
        let filter_row = self.filter_active;

        // Precompute staged counts per directory
        let dir_counts = self.dir_staged_counts();

        let mut lines: Vec<Line> = Vec::new();
        let mut highlights: Vec<(usize, Style)> = Vec::new();

        for (i, item) in self.visible.iter().enumerate() {
            let (line, staged_all) = self.item_line(item, &dir_counts);
            if staged_all && i != self.selection {
                highlights.push((i, Style::default().bg(theme.staged_bg)));
            }
            lines.push(line);
        }

        let view_height = content_height.saturating_sub(if filter_row { 1 } else { 0 });
        // keep selection visible with minimal scrolling
        let mut scroll = self.scroll.get();
        if view_height > 0 {
            if self.selection < scroll {
                scroll = self.selection;
            } else if self.selection >= scroll + view_height {
                scroll = self.selection - view_height + 1;
            }
        }
        scroll = ui::clamp_scroll(scroll, lines.len(), view_height);
        self.scroll.set(scroll);

        if !lines.is_empty() {
            let mut hl = highlights.clone();
            hl.push((
                self.selection,
                Style::default()
                    .bg(theme.selection_bg)
                    .add_modifier(Modifier::BOLD),
            ));
            ui::render_lines(f, inner, &lines, scroll, &hl);
        } else {
            let msg = if self.pending {
                strings::MSG.loading
            } else {
                strings::MSG.empty_status
            };
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(msg, theme.dim))),
                inner,
            );
        }

        // Filter input row
        if filter_row {
            let y = inner.y + inner.height - 1;
            ui::render_line_at(
                f,
                inner.x,
                y,
                inner.width,
                &Line::from(vec![
                    Span::styled("filter> ", theme.info),
                    Span::raw(self.filter.clone()),
                ]),
            );
        }
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        self.event(ev)
    }
}

impl StatusTreeComponent {
    fn item_line(
        &self,
        item: &TreeItem,
        dir_counts: &std::collections::HashMap<String, (usize, usize)>,
    ) -> (Line<'static>, bool) {
        let theme = &self.ctx.theme;
        let mut spans: Vec<Span> = Vec::new();
        let indent = item.depth;
        spans.push(Span::raw("  ".repeat(indent)));

        match &item.kind {
            TreeItemKind::Dir { expanded } => {
                let sym = if *expanded { "▼ " } else { "▶ " };
                spans.push(Span::styled(
                    format!("{sym}{}/", item.name),
                    Style::default()
                        .fg(theme.text.fg.unwrap_or(ratatui::style::Color::Gray))
                        .add_modifier(Modifier::BOLD),
                ));
                let (staged, total) = dir_counts.get(&item.path).copied().unwrap_or((0, 0));
                let staged_all = total > 0 && staged == total;
                (Line::from(spans), staged_all)
            }
            TreeItemKind::File { entry } => {
                let e = self.entries.get(*entry);
                if let Some(e) = e {
                    let code = if e.tree_conflict == 'C' {
                        'C'
                    } else {
                        e.status
                    };
                    spans.push(Span::styled(code.to_string(), theme.status_style(code)));
                    spans.push(Span::raw(" "));
                    if e.tree_conflict == 'C' {
                        spans.push(Span::styled("C ", theme.status_conflicted));
                    }
                } else {
                    spans.push(Span::raw("  "));
                }
                let name = item.name.clone();
                let staged = self.staged.contains(&item.path);
                let style = if staged {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                spans.push(Span::styled(name, style));
                (Line::from(spans), staged)
            }
        }
    }

    fn dir_staged_counts(&self) -> std::collections::HashMap<String, (usize, usize)> {
        let mut counts: std::collections::HashMap<String, (usize, usize)> =
            std::collections::HashMap::new();
        for e in &self.entries {
            if e.is_dir {
                continue;
            }
            // walk ancestors
            let mut prefix = String::new();
            for part in e.path.split('/') {
                if !prefix.is_empty() {
                    prefix.push('/');
                }
                prefix.push_str(part);
                if prefix == e.path {
                    break;
                }
                let c = counts.entry(prefix.clone()).or_insert((0, 0));
                c.1 += 1;
                if self.staged.contains(&e.path) {
                    c.0 += 1;
                }
            }
        }
        counts
    }
}

// ----- tree building (module level, testable) -----

fn build_tree(entries: &[StatusEntry]) -> Vec<Node> {
    let mut roots: Vec<Node> = Vec::new();
    for (idx, e) in entries.iter().enumerate() {
        let parts: Vec<&str> = e.path.split('/').collect();
        let mut level = &mut roots;
        let mut cur = String::new();
        for (i, part) in parts.iter().enumerate() {
            if !cur.is_empty() {
                cur.push('/');
            }
            cur.push_str(part);
            let is_last = i == parts.len() - 1;
            if let Some(pos) = level.iter().position(|n| n.name == *part) {
                if is_last {
                    level[pos].entry = Some(idx);
                }
                level = &mut level[pos].children;
            } else {
                level.push(Node {
                    name: part.to_string(),
                    path: cur.clone(),
                    is_dir: !is_last || e.is_dir,
                    entry: if is_last { Some(idx) } else { None },
                    children: Vec::new(),
                });
                let last = level.last_mut().expect("just pushed");
                level = &mut last.children;
            }
        }
    }
    fn sort_nodes(nodes: &mut [Node]) {
        nodes.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
        for n in nodes.iter_mut() {
            sort_nodes(&mut n.children);
        }
    }
    sort_nodes(&mut roots);
    roots
}

fn flatten(nodes: &[Node], expanded: &HashSet<String>, depth: usize, out: &mut Vec<TreeItem>) {
    for node in nodes {
        let is_expanded = expanded.contains(&node.path);
        out.push(TreeItem {
            depth,
            path: node.path.clone(),
            name: node.name.clone(),
            kind: if node.is_dir {
                TreeItemKind::Dir {
                    expanded: is_expanded,
                }
            } else {
                TreeItemKind::File {
                    entry: node.entry.unwrap_or(0),
                }
            },
        });
        if node.is_dir && is_expanded {
            flatten(&node.children, expanded, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svn::models::StatusEntry;
    use crate::ui::style::Theme;

    fn entry(status: char, path: &str, is_dir: bool) -> StatusEntry {
        StatusEntry {
            status,
            props_status: ' ',
            tree_conflict: ' ',
            path: path.to_string(),
            is_dir,
        }
    }

    #[test]
    fn test_build_tree_and_flatten() {
        let entries = vec![
            entry('M', "Cargo.toml", false),
            entry('?', "newfile.txt", false),
            entry('M', "src/main.rs", false),
            entry('A', "src/lib/foo.rs", false),
        ];
        let nodes = build_tree(&entries);
        assert_eq!(nodes.len(), 3); // src, Cargo.toml, newfile.txt
        let src = nodes.iter().find(|n| n.name == "src").unwrap();
        assert!(src.is_dir);
        let mut expanded = HashSet::new();
        expanded.insert("src".to_string());
        let mut visible = Vec::new();
        flatten(&nodes, &expanded, 0, &mut visible);
        assert_eq!(visible.len(), 5);
        // dirs first
        assert_eq!(visible[0].name, "src");
        assert_eq!(visible[0].depth, 0);
        let main = visible.iter().find(|t| t.path == "src/main.rs").unwrap();
        assert_eq!(main.depth, 1);
        let lib = visible.iter().find(|t| t.path == "src/lib").unwrap();
        assert!(matches!(lib.kind, TreeItemKind::Dir { .. }));
        // collapsed: src children hidden
        let mut visible2 = Vec::new();
        flatten(&nodes, &HashSet::new(), 0, &mut visible2);
        assert_eq!(visible2.len(), 3);
    }

    #[test]
    fn test_stage_toggle() {
        let mut comp = StatusTreeComponent {
            ctx: Context {
                queue: crate::queue::Queue::new(),
                theme: Theme::default(),
            },
            entries: vec![entry('M', "a.txt", false)],
            visible: vec![TreeItem {
                depth: 0,
                path: "a.txt".to_string(),
                name: "a.txt".to_string(),
                kind: TreeItemKind::File { entry: 0 },
            }],
            expanded: HashSet::new(),
            selection: 0,
            scroll: std::cell::Cell::new(0),
            staged: HashSet::new(),
            filter: String::new(),
            filter_active: false,
            pending: false,
            focused: true,
        };
        let (added, removed) = comp.toggle_stage_at_selection();
        assert_eq!(added, vec!["a.txt"]);
        assert!(removed.is_empty());
        let (added, removed) = comp.toggle_stage_at_selection();
        assert!(added.is_empty());
        assert_eq!(removed, vec!["a.txt"]);
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::queue::InternalEvent;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn entry(status: char, path: &str) -> StatusEntry {
        crate::svn::models::StatusEntry {
            status,
            props_status: ' ',
            tree_conflict: ' ',
            path: path.to_string(),
            is_dir: std::path::Path::new(path).is_dir(),
        }
    }

    fn comp_with(entries: Vec<StatusEntry>) -> (StatusTreeComponent, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = crate::components::Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut c = StatusTreeComponent::new(&ctx);
        c.update(entries);
        (c, q)
    }

    fn key(code: crossterm::event::KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(
            code,
            crossterm::event::KeyModifiers::NONE,
        ))
    }

    #[test]
    fn navigation_moves_selection() {
        let (mut c, _q) = comp_with(vec![entry('M', "a.txt"), entry('?', "b.txt")]);
        // files sorted: a.txt, b.txt
        assert_eq!(c.selection_path().as_deref(), Some("a.txt"));
        assert!(c.event(&key(KeyCode::Char('j'))).unwrap().consumed);
        assert_eq!(c.selection_path().as_deref(), Some("b.txt"));
        c.event(&key(KeyCode::Char('j'))).unwrap(); // clamp at end
        assert_eq!(c.selection_path().as_deref(), Some("b.txt"));
        c.event(&key(KeyCode::Char('k'))).unwrap();
        assert_eq!(c.selection_path().as_deref(), Some("a.txt"));
        c.event(&key(KeyCode::End)).unwrap();
        assert_eq!(c.selection_path().as_deref(), Some("b.txt"));
        c.event(&key(KeyCode::Home)).unwrap();
        assert_eq!(c.selection_path().as_deref(), Some("a.txt"));
        // arrow keys work too
        c.event(&key(KeyCode::Down)).unwrap();
        assert_eq!(c.selection_path().as_deref(), Some("b.txt"));
        // unknown keys are not consumed
        assert!(!c.event(&key(KeyCode::Char('q'))).unwrap().consumed);
    }

    #[test]
    fn expand_collapse_dir() {
        let (mut c, _q) = comp_with(vec![entry('M', "src/main.rs"), entry('M', "a.txt")]);
        // dirs first: src, then a.txt
        assert_eq!(c.selection_path().as_deref(), Some("src"));
        // expand with l / right arrow
        c.event(&key(KeyCode::Char('l'))).unwrap();
        assert_eq!(c.visible.len(), 3);
        assert!(matches!(
            c.visible[0].kind,
            TreeItemKind::Dir { expanded: true }
        ));
        c.event(&key(KeyCode::Right)).unwrap(); // no-op on expanded
        // collapse with h
        c.event(&key(KeyCode::Char('h'))).unwrap();
        assert_eq!(c.visible.len(), 2);
        assert!(matches!(
            c.visible[0].kind,
            TreeItemKind::Dir { expanded: false }
        ));
        // expand with Enter
        c.event(&key(KeyCode::Enter)).unwrap();
        assert_eq!(c.visible.len(), 3);
        // Enter on a file requests a fullscreen diff
        c.event(&key(KeyCode::Char('j'))).unwrap(); // src/main.rs
        c.event(&key(KeyCode::Char('j'))).unwrap(); // a.txt
        c.event(&key(KeyCode::Enter)).unwrap();
    }

    #[test]
    fn staging_toggles_and_requests_add() {
        let (mut c, q) = comp_with(vec![entry('M', "a.txt"), entry('?', "b.txt")]);
        // stage a.txt (modified): no svn add needed
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.contains("a.txt"));
        assert!(q.pop().is_none());
        // stage b.txt (unversioned): pushes AddFiles
        c.event(&key(KeyCode::Char('j'))).unwrap();
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.contains("b.txt"));
        match q.pop() {
            Some(InternalEvent::AddFiles(paths)) => assert_eq!(paths, vec!["b.txt"]),
            other => panic!("expected AddFiles, got {other:?}"),
        }
        // unstage
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(!c.staged.contains("b.txt"));
    }

    #[test]
    fn dir_staging_stages_descendants() {
        let (mut c, _q) = comp_with(vec![entry('M', "src/main.rs"), entry('M', "src/lib.rs")]);
        assert_eq!(c.selection_path().as_deref(), Some("src"));
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.contains("src/main.rs"));
        assert!(c.staged.contains("src/lib.rs"));
        assert_eq!(c.staged_count(), 2);
        // toggle off removes both
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.is_empty());
    }

    #[test]
    fn add_and_revert_keys() {
        let (mut c, q) = comp_with(vec![entry('?', "b.txt")]);
        // 'a' adds unversioned files
        c.event(&key(KeyCode::Char('a'))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::AddFiles(p)) if p == vec!["b.txt"]));
        // 'r' on unversioned → info message (no revert confirm)
        c.event(&key(KeyCode::Char('r'))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ShowInfoMsg(_))));
        // 'r' on modified → confirm popup
        let (mut c2, q2) = comp_with(vec![entry('M', "m.txt")]);
        c2.event(&key(KeyCode::Char('r'))).unwrap();
        assert!(matches!(
            q2.pop(),
            Some(InternalEvent::Confirm(crate::queue::ConfirmAction::Revert(p)))
                if p == vec!["m.txt"]
        ));
    }

    #[test]
    fn commit_update_resolve_keys() {
        let (mut c, q) = comp_with(vec![entry('C', "c.txt")]);
        c.event(&key(KeyCode::Char('c'))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::OpenCommit)));
        c.event(&key(KeyCode::Char('u'))).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Confirm(crate::queue::ConfirmAction::Update))
        ));
        c.event(&key(KeyCode::Char('x'))).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Confirm(crate::queue::ConfirmAction::Resolve(p)))
                if p == "c.txt"
        ));
        // 'x' on non-conflicted file is a no-op
        let (mut c2, q2) = comp_with(vec![entry('M', "m.txt")]);
        c2.event(&key(KeyCode::Char('x'))).unwrap();
        assert!(q2.pop().is_none());
    }

    #[test]
    fn filter_mode_captures_typing() {
        let (mut c, _q) = comp_with(vec![entry('M', "alpha.txt"), entry('?', "beta.txt")]);
        c.event(&key(KeyCode::Char('/'))).unwrap();
        assert!(c.filter_active);
        // typing goes into the filter
        c.event(&key(KeyCode::Char('a'))).unwrap();
        c.event(&key(KeyCode::Char('l'))).unwrap();
        assert_eq!(c.filter, "al");
        assert_eq!(c.visible.len(), 1);
        assert_eq!(c.visible[0].path, "alpha.txt");
        // backspace
        c.event(&key(KeyCode::Backspace)).unwrap();
        assert_eq!(c.filter, "a");
        assert_eq!(c.visible.len(), 2);
        // Esc exits filter mode
        c.event(&key(KeyCode::Esc)).unwrap();
        assert!(!c.filter_active);
        assert_eq!(c.visible.len(), 2);
    }

    #[test]
    fn refresh_and_tab_keys() {
        let (mut c, q) = comp_with(vec![entry('M', "a.txt")]);
        c.event(&key(KeyCode::F(5))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::RefreshStatus)));
        c.event(&key(KeyCode::Char('2'))).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::SwitchTab(crate::queue::Tab::Log))
        ));
        c.event(&key(KeyCode::Char('1'))).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::SwitchTab(crate::queue::Tab::Status))
        ));
        c.event(&key(KeyCode::Char('?'))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::OpenHelp)));
    }

    #[test]
    fn draw_renders_entries_and_highlights() {
        let (c, _q) = comp_with(vec![entry('M', "src/main.rs"), entry('?', "b.txt")]);
        let terminal = ts::render(60, 10, |f| {
            c.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        let s = ts::dump(&terminal);
        assert!(s.contains("src"), "{s}");
        assert!(s.contains("b.txt"), "{s}");
        assert!(s.contains("Files (svn status)"), "{s}");
    }

    #[test]
    fn draw_loading_and_empty() {
        let ctx = crate::components::Context {
            queue: crate::queue::Queue::new(),
            theme: Theme::default(),
        };
        let mut c = StatusTreeComponent::new(&ctx);
        // empty + pending → Loading...
        let t1 = ts::render(40, 6, |f| {
            c.draw(f, Rect::new(0, 0, 40, 6)).unwrap();
        });
        assert!(ts::dump(&t1).contains("Loading"));
        // update with empty entries → working copy is clean
        c.update(vec![]);
        let t2 = ts::render(40, 6, |f| {
            c.draw(f, Rect::new(0, 0, 40, 6)).unwrap();
        });
        assert!(ts::dump(&t2).contains("clean"), "{}", ts::dump(&t2));
    }

    #[test]
    fn selection_survives_update() {
        let (mut c, _q) = comp_with(vec![entry('M', "src/main.rs"), entry('M', "a.txt")]);
        // expand src, then move to src/main.rs
        c.event(&key(KeyCode::Char('l'))).unwrap();
        c.event(&key(KeyCode::Char('j'))).unwrap();
        assert_eq!(c.selection_path().as_deref(), Some("src/main.rs"));
        c.update(vec![
            entry('M', "src/main.rs"),
            entry('M', "a.txt"),
            entry('M', "c.txt"),
        ]);
        assert_eq!(c.selection_path().as_deref(), Some("src/main.rs"));
    }

    #[test]
    fn paths_at_selection_for_file_and_dir() {
        let (mut c, _q) = comp_with(vec![
            entry('M', "src/main.rs"),
            entry('M', "src/lib.rs"),
            entry('M', "top.txt"),
        ]);
        assert_eq!(c.paths_at_selection(), vec!["src/main.rs", "src/lib.rs"]);
        c.event(&key(KeyCode::End)).unwrap();
        assert_eq!(c.paths_at_selection(), vec!["top.txt"]);
    }
}
