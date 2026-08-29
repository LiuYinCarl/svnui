//! Status file tree: shows `svn status` entries grouped by directory,
//! supports staging (commit set), filtering, and selection-driven diff.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::{ConfirmAction, InternalEvent};
use crate::strings::{self, TITLE};
use crate::svn::models::{StatusEntry, TreeItem, TreeItemKind};
use crate::ui;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use std::collections::{HashMap, HashSet};

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
    pub pending: bool,
    pub focused: bool,
    /// Cache of per-directory (staged, total) file counts, recomputed only
    /// when the staged set or the status entries change (not per draw).
    counts_cache: std::cell::RefCell<std::collections::HashMap<String, (usize, usize)>>,
    counts_dirty: std::cell::Cell<bool>,
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
            pending: true,
            focused: true,
            counts_cache: std::cell::RefCell::new(std::collections::HashMap::new()),
            counts_dirty: std::cell::Cell::new(true),
        }
    }

    // ----- public API used by the app -----

    pub fn update(&mut self, entries: Vec<StatusEntry>) {
        self.pending = false;
        // Prune staged paths that no longer have a status entry (committed
        // or reverted elsewhere) so they cannot poison the next commit.
        if !self.staged.is_empty() {
            let present: HashSet<&str> = entries.iter().map(|e| e.path.as_str()).collect();
            self.staged.retain(|p| present.contains(p.as_str()));
        }
        self.entries = entries;
        self.counts_dirty.set(true);
        self.rebuild_visible();
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// Number of currently visible tree items (files + dirs).
    pub fn visible_len(&self) -> usize {
        self.visible.len()
    }

    /// The active path-substring filter (empty = no filtering).
    pub fn filter(&self) -> &str {
        &self.filter
    }

    /// Set the path-substring filter and rebuild the visible list. Called
    /// live while typing in the filter popup, and with an empty string to
    /// clear the filter.
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.rebuild_visible();
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

    /// Paths affected by staging the item under the cursor. For a directory
    /// this includes the directory's own status entry (if any — e.g. an
    /// unversioned or property-changed dir) plus all descendant files; svn
    /// accepts directories as add/commit/revert targets.
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
    ///
    /// Dir-toggle semantics: if anything stageable under the selection is
    /// unstaged, stage it all (complete, don't invert); only when everything
    /// is staged does the toggle unstage. Missing ('!'), obstructed ('~')
    /// and conflicted entries cannot be committed and are never staged.
    pub fn toggle_stage_at_selection(&mut self) -> (Vec<String>, Vec<String>) {
        let paths = self.paths_at_selection();
        if paths.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let by_path = self.entries_by_path();
        let stageable_paths: Vec<&String> = paths
            .iter()
            .filter(|p| by_path.get(p.as_str()).is_none_or(|e| stageable(e)))
            .collect();
        if stageable_paths.iter().all(|p| self.staged.contains(*p)) {
            // Everything stageable is already staged → unstage everything
            // under the selection. This also clears staged paths that have
            // since turned unstageable (e.g. the file went missing).
            let removed: Vec<String> = paths
                .iter()
                .filter(|p| self.staged.contains(*p))
                .cloned()
                .collect();
            if !removed.is_empty() {
                self.unset_staged(&paths);
                return (Vec::new(), removed);
            }
        }
        if stageable_paths.is_empty() {
            self.ctx.queue.push(InternalEvent::ShowInfoMsg(
                "Nothing stageable here (missing/obstructed/conflicted files must be fixed first)"
                    .to_string(),
            ));
            return (Vec::new(), Vec::new());
        }
        let added: Vec<String> = stageable_paths
            .iter()
            .filter(|p| !self.staged.contains(**p))
            .map(|p| (*p).clone())
            .collect();
        self.set_staged(&added);
        (added, Vec::new())
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
        if !added.is_empty() || !removed.is_empty() {
            self.counts_dirty.set(true);
        }
        (added, removed)
    }

    pub fn set_staged(&mut self, paths: &[String]) {
        for p in paths {
            self.staged.insert(p.clone());
        }
        if !paths.is_empty() {
            self.counts_dirty.set(true);
        }
    }

    pub fn unset_staged(&mut self, paths: &[String]) {
        for p in paths {
            self.staged.remove(p);
        }
        if !paths.is_empty() {
            self.counts_dirty.set(true);
        }
    }

    pub fn clear_staged(&mut self) {
        if !self.staged.is_empty() {
            self.counts_dirty.set(true);
        }
        self.staged.clear();
    }

    pub fn staged_count(&self) -> usize {
        self.staged.len()
    }

    /// All changed files as (status char, path), sorted by path.
    pub fn changed_files(&self) -> Vec<(char, String)> {
        let mut v: Vec<(char, String)> = self
            .entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| (e.status, e.path.clone()))
            .collect();
        v.sort_by(|a, b| a.1.cmp(&b.1));
        v
    }

    /// Status char of a path, ' ' when the path has no status entry.
    pub fn status_char(&self, path: &str) -> char {
        self.entry_for_path(path).map(|e| e.status).unwrap_or(' ')
    }

    /// Staged files as (status char, path), sorted by path.
    pub fn staged_files(&self) -> Vec<(char, String)> {
        let mut v: Vec<(char, String)> = self
            .staged
            .iter()
            .map(|p| {
                let s = self.entry_for_path(p).map(|e| e.status).unwrap_or('?');
                (s, p.clone())
            })
            .collect();
        v.sort_by(|a, b| a.1.cmp(&b.1));
        v
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    // ----- internals -----

    /// Paths affected by acting on a directory: the directory's own status
    /// entry (if any) plus all descendant file entries. The dir's own entry
    /// matters for unversioned dirs (`? newdir` has no children in
    /// `svn status` output) and dirs with property-only changes.
    fn files_under(&self, dir_path: &str) -> Vec<String> {
        let prefix = format!("{dir_path}/");
        self.entries
            .iter()
            .filter(|e| e.path == dir_path || (!e.is_dir && e.path.starts_with(&prefix)))
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
                let by_path = self.entries_by_path();
                let needs_add: Vec<String> = added
                    .iter()
                    .filter(|p| by_path.get(p.as_str()).is_some_and(|e| e.status == '?'))
                    .cloned()
                    .collect();
                if !needs_add.is_empty() {
                    self.ctx.queue.push(InternalEvent::AddFiles(needs_add));
                }
            }
        } else if key_match(k, KeyAction::StageAll) {
            // One pass over the entries: no per-path lookups, and files svn
            // cannot commit (missing/obstructed/conflicted) are skipped so
            // the next commit does not fail wholesale.
            let mut all = Vec::new();
            let mut needs_add = Vec::new();
            let mut skipped = 0usize;
            for e in &self.entries {
                if e.is_dir {
                    continue;
                }
                if !stageable(e) {
                    skipped += 1;
                    continue;
                }
                if e.status == '?' {
                    needs_add.push(e.path.clone());
                }
                all.push(e.path.clone());
            }
            self.set_staged(&all);
            if !needs_add.is_empty() {
                self.ctx.queue.push(InternalEvent::AddFiles(needs_add));
            }
            if skipped > 0 {
                self.ctx.queue.push(InternalEvent::ShowInfoMsg(format!(
                    "Skipped {skipped} missing/obstructed/conflicted file(s)"
                )));
            }
        } else if key_match(k, KeyAction::UnstageAll) {
            self.clear_staged();
        } else if key_match(k, KeyAction::AddFiles) {
            let paths = self.paths_at_selection();
            if !paths.is_empty() {
                let by_path = self.entries_by_path();
                let unversioned: Vec<String> = paths
                    .iter()
                    .filter(|p| by_path.get(p.as_str()).is_none_or(|e| e.status == '?'))
                    .cloned()
                    .collect();
                if !unversioned.is_empty() {
                    self.ctx.queue.push(InternalEvent::AddFiles(unversioned));
                }
            }
        } else if key_match(k, KeyAction::RevertFiles) {
            let by_path = self.entries_by_path();
            let paths: Vec<String> = self
                .paths_at_selection()
                .into_iter()
                .filter(|p| by_path.get(p.as_str()).is_some_and(|e| e.status != '?'))
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
            self.ctx.queue.push(InternalEvent::OpenStatusFilter);
        } else if key_match(k, KeyAction::Escape) {
            // with an active filter Esc clears it; without one Esc is left
            // unconsumed for the caller (e.g. commit pane focus return)
            if self.filter.is_empty() {
                return Ok(EventState::not_consumed());
            }
            self.set_filter(String::new());
        } else if key_match(k, KeyAction::DiffFull) {
            if self.selection_entry().is_some() {
                self.ctx.queue.push(InternalEvent::RequestFileDiff);
            }
        } else if key_match(k, KeyAction::Blame) {
            if let Some(e) = self.selection_entry()
                && !e.is_dir
            {
                self.ctx
                    .queue
                    .push(InternalEvent::RequestBlame(e.path.clone()));
            }
        } else if key_match(k, KeyAction::FileHistory) {
            if let Some(e) = self.selection_entry()
                && !e.is_dir
            {
                self.ctx.queue.push(InternalEvent::RequestFileHistory);
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

    /// Path → entry index map, built once per event so handlers that touch
    /// many paths stay O(n) instead of doing a linear scan per path.
    fn entries_by_path(&self) -> HashMap<&str, &StatusEntry> {
        self.entries.iter().map(|e| (e.path.as_str(), e)).collect()
    }
}

/// Whether an entry can be committed as-is. Missing ('!') and obstructed
/// ('~') files make `svn commit` fail wholesale; conflicted files must be
/// resolved first.
fn stageable(e: &StatusEntry) -> bool {
    !matches!(e.status, '!' | '~') && !e.is_conflicted()
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

        let view_height = inner.height as usize;

        // Precompute staged counts per directory (cached: only recomputed
        // when the staged set or the status entries change).
        let dir_counts = self.dir_staged_counts();

        let total = self.visible.len();
        // keep selection visible with minimal scrolling
        let mut scroll = self.scroll.get();
        if view_height > 0 {
            if self.selection < scroll {
                scroll = self.selection;
            } else if self.selection >= scroll + view_height {
                scroll = self.selection - view_height + 1;
            }
        }
        scroll = ui::clamp_scroll(scroll, total, view_height);
        self.scroll.set(scroll);

        if total > 0 {
            // Virtualized rendering: only build the visible window of lines,
            // so drawing a 100k-entry tree costs O(screen height), not O(n).
            let end = (scroll + view_height).min(total);
            let mut lines: Vec<Line> = Vec::with_capacity(end - scroll);
            let mut highlights: Vec<(usize, Style)> = Vec::new();
            for i in scroll..end {
                let item = &self.visible[i];
                let (line, staged_all) = self.item_line(item, &dir_counts);
                if staged_all && i != self.selection {
                    highlights.push((i - scroll, Style::default().bg(theme.staged_bg)));
                }
                lines.push(line);
            }
            if self.selection >= scroll && self.selection < end {
                highlights.push((
                    self.selection - scroll,
                    Style::default()
                        .bg(theme.selection_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            ui::render_lines(f, inner, &lines, 0, &highlights);
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

    fn dir_staged_counts(
        &self,
    ) -> std::cell::Ref<'_, std::collections::HashMap<String, (usize, usize)>> {
        if self.counts_dirty.replace(false) {
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
            *self.counts_cache.borrow_mut() = counts;
        }
        self.counts_cache.borrow()
    }
}

// ----- tree building (module level, testable) -----
//
// `build_tree` must scale to very large working copies (100k+ files). The
// implementation is O(n): nodes are created once in a HashMap keyed by path,
// then assembled into parent/child relationships by index, then sorted.

fn build_tree(entries: &[StatusEntry]) -> Vec<Node> {
    use std::collections::HashMap;

    // 1. create every node exactly once (files and intermediate dirs)
    let mut nodes: HashMap<String, Node> = HashMap::new();
    for (idx, e) in entries.iter().enumerate() {
        let mut cur = String::new();
        for (i, part) in e.path.split('/').enumerate() {
            if i > 0 {
                cur.push('/');
            }
            cur.push_str(part);
            let is_last = i + 1 == e.path.split('/').count();
            let entry = if is_last { Some(idx) } else { None };
            let is_dir = !is_last || e.is_dir;
            match nodes.entry(cur.clone()) {
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    if is_last {
                        let n = o.get_mut();
                        n.entry = entry;
                        n.is_dir = is_dir;
                    }
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(Node {
                        name: part.to_string(),
                        path: cur.clone(),
                        is_dir,
                        entry,
                        children: Vec::new(),
                    });
                }
            }
        }
    }

    // 2. assemble children by path index
    let paths: Vec<String> = nodes.keys().cloned().collect();
    let index: HashMap<&str, usize> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| (p.as_str(), i))
        .collect();
    let mut flat: Vec<Option<Node>> = paths.iter().map(|p| nodes.remove(p)).collect();
    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); flat.len()];
    let mut roots_idx: Vec<usize> = Vec::new();
    for (i, n) in flat.iter().enumerate() {
        let Some(n) = n else { continue };
        match n.path.rfind('/') {
            Some(pos) => match index.get(&n.path[..pos]) {
                Some(&pi) => children_of[pi].push(i),
                None => roots_idx.push(i),
            },
            None => roots_idx.push(i),
        }
    }

    fn collect(i: usize, flat: &mut [Option<Node>], children_of: &[Vec<usize>]) -> Option<Node> {
        // each node is taken exactly once (one parent prefix per path);
        // a repeat visit would mean corrupt assembly — skip the node
        // instead of panicking
        let mut node = flat[i].take()?;
        node.children = children_of[i]
            .iter()
            .filter_map(|&c| collect(c, flat, children_of))
            .collect();
        // A node with children is structurally a directory, even when the
        // disk probe said otherwise — after `svn rm dir` the dir is gone
        // from disk but status still lists it and its deleted children, and
        // only dir nodes are expandable (so their children stay reachable).
        node.is_dir = node.is_dir || !node.children.is_empty();
        Some(node)
    }

    let mut roots: Vec<Node> = roots_idx
        .iter()
        .filter_map(|&i| collect(i, &mut flat, &children_of))
        .collect();

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
            pending: false,
            focused: true,
            counts_cache: std::cell::RefCell::new(std::collections::HashMap::new()),
            counts_dirty: std::cell::Cell::new(true),
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
    use crossterm::event::KeyCode;

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
    fn stage_all_and_unstage_all_keys() {
        let (mut c, q) = comp_with(vec![
            entry('M', "a.txt"),
            entry('?', "new.txt"),
            entry('M', "dir/b.txt"),
        ]);
        // A stages everything; unversioned files are svn-added
        c.event(&key(KeyCode::Char('A'))).unwrap();
        assert_eq!(c.staged_count(), 3);
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::AddFiles(paths)) if paths == vec!["new.txt".to_string()]
        ));
        // U clears the commit set
        c.event(&key(KeyCode::Char('U'))).unwrap();
        assert_eq!(c.staged_count(), 0);
        assert!(q.pop().is_none());
    }

    #[test]
    fn filter_via_set_filter_and_slash_opens_popup() {
        let (mut c, q) = comp_with(vec![entry('M', "alpha.txt"), entry('?', "beta.txt")]);
        // '/' asks the app to open the filter popup
        c.event(&key(KeyCode::Char('/'))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::OpenStatusFilter)));
        assert!(c.filter().is_empty());
        // set_filter filters by path substring (case-insensitive)
        c.set_filter("AL".to_string());
        assert_eq!(c.visible.len(), 1);
        assert_eq!(c.visible[0].path, "alpha.txt");
        // narrowing further keeps only matches
        c.set_filter("alpha".to_string());
        assert_eq!(c.visible.len(), 1);
        // clearing restores the full list
        c.set_filter(String::new());
        assert_eq!(c.visible.len(), 2);
    }

    #[test]
    fn esc_clears_active_filter() {
        let (mut c, _q) = comp_with(vec![entry('M', "alpha.txt"), entry('?', "beta.txt")]);
        c.set_filter("alpha".to_string());
        assert_eq!(c.visible.len(), 1);
        // Esc with an active filter clears it and is consumed
        assert!(c.event(&key(KeyCode::Esc)).unwrap().consumed);
        assert!(c.filter().is_empty());
        assert_eq!(c.visible.len(), 2);
        // Esc without a filter is not consumed
        assert!(!c.event(&key(KeyCode::Esc)).unwrap().consumed);
    }

    #[test]
    fn title_shows_active_filter() {
        let (mut c, _q) = comp_with(vec![entry('M', "alpha.txt"), entry('?', "beta.txt")]);
        c.set_filter("foo".to_string());
        let t = ts::render(60, 10, |f| {
            c.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("Files (svn status)"), "{s}");
        assert!(s.contains("filter: \"foo\""), "{s}");
        // filter cleared → plain title again
        c.set_filter(String::new());
        let t2 = ts::render(60, 10, |f| {
            c.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        let s2 = ts::dump(&t2);
        assert!(!s2.contains("filter:"), "{s2}");
    }

    #[test]
    fn draw_with_zero_height_area_does_not_panic() {
        let (mut c, _q) = comp_with(vec![entry('M', "a.txt")]);
        c.set_filter("a".to_string()); // filter shown in the title
        let _t = ts::render(40, 5, |f| {
            c.draw(f, Rect::new(0, 0, 40, 0)).unwrap();
        });
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

    /// A status entry for a directory (is_dir on disk).
    fn dir_entry(status: char, path: &str) -> StatusEntry {
        crate::svn::models::StatusEntry {
            status,
            props_status: ' ',
            tree_conflict: ' ',
            path: path.to_string(),
            is_dir: true,
        }
    }

    #[test]
    fn deleted_dir_with_children_is_expandable() {
        // after `svn rm gone` the dir no longer exists on disk (is_dir=false)
        // but status still lists it and its deleted children; the node must
        // still behave as a directory or the children are unreachable
        let (mut c, _q) = comp_with(vec![
            entry('D', "gone"),
            entry('D', "gone/a.txt"),
            entry('D', "gone/b.txt"),
        ]);
        assert!(matches!(c.visible[0].kind, TreeItemKind::Dir { .. }));
        c.event(&key(KeyCode::Char('l'))).unwrap(); // expand
        assert_eq!(c.visible.len(), 3);
        assert_eq!(c.visible[1].path, "gone/a.txt");
        c.event(&key(KeyCode::Char('h'))).unwrap(); // collapse
        // staging the dir covers the dir itself and all deleted children
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.contains("gone"));
        assert!(c.staged.contains("gone/a.txt"));
        assert!(c.staged.contains("gone/b.txt"));
    }

    #[test]
    fn unversioned_dir_can_be_staged() {
        // svn does not recurse into unversioned dirs, so `? newdir` has no
        // children; the dir's own path must be stageable
        let (mut c, q) = comp_with(vec![dir_entry('?', "newdir")]);
        assert_eq!(c.paths_at_selection(), vec!["newdir"]);
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.contains("newdir"));
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::AddFiles(p)) if p == vec!["newdir"]
        ));
    }

    #[test]
    fn dir_with_own_entry_stages_dir_and_children() {
        let (mut c, _q) = comp_with(vec![dir_entry('M', "dir"), entry('M', "dir/f.txt")]);
        assert_eq!(c.paths_at_selection(), vec!["dir", "dir/f.txt"]);
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.contains("dir"));
        assert!(c.staged.contains("dir/f.txt"));
    }

    #[test]
    fn missing_obstructed_conflicted_files_are_not_staged() {
        for status in ['!', '~', 'C'] {
            let (mut c, q) = comp_with(vec![entry(status, "bad.txt")]);
            c.event(&key(KeyCode::Char(' '))).unwrap();
            assert!(c.staged.is_empty(), "status {status} must not stage");
            assert!(matches!(q.pop(), Some(InternalEvent::ShowInfoMsg(_))));
        }
        // a tree-conflict flag also refuses staging
        let mut e = entry('M', "conf.txt");
        e.tree_conflict = 'C';
        let (mut c, q) = comp_with(vec![e]);
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.is_empty());
        assert!(matches!(q.pop(), Some(InternalEvent::ShowInfoMsg(_))));
        // unstaging a previously staged bad path still works
        let (mut c, _q) = comp_with(vec![entry('M', "f.txt")]);
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.contains("f.txt"));
        c.update(vec![entry('!', "f.txt")]);
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.is_empty());
    }

    #[test]
    fn stage_all_skips_missing_obstructed_conflicted() {
        let (mut c, q) = comp_with(vec![
            entry('M', "ok.txt"),
            entry('!', "missing.txt"),
            entry('~', "obstructed.txt"),
            entry('C', "conflicted.txt"),
        ]);
        c.event(&key(KeyCode::Char('A'))).unwrap();
        assert_eq!(c.staged_count(), 1);
        assert!(c.staged.contains("ok.txt"));
        assert!(matches!(q.pop(), Some(InternalEvent::ShowInfoMsg(_))));
    }

    #[test]
    fn update_prunes_staged_paths_that_disappeared() {
        let (mut c, _q) = comp_with(vec![entry('M', "a.txt"), entry('M', "b.txt")]);
        c.set_staged(&["a.txt".to_string(), "b.txt".to_string()]);
        // b.txt is no longer changed (committed or reverted elsewhere)
        c.update(vec![entry('M', "a.txt")]);
        assert!(c.staged.contains("a.txt"));
        assert!(!c.staged.contains("b.txt"));
        assert_eq!(c.staged_count(), 1);
    }

    #[test]
    fn partially_staged_dir_toggle_completes_then_unstages() {
        let (mut c, _q) = comp_with(vec![entry('M', "src/main.rs"), entry('M', "src/lib.rs")]);
        // one file already staged: space on the dir completes the set
        // instead of inverting each descendant
        c.set_staged(&["src/main.rs".to_string()]);
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.contains("src/main.rs"));
        assert!(c.staged.contains("src/lib.rs"));
        // fully staged: space unstages everything
        c.event(&key(KeyCode::Char(' '))).unwrap();
        assert!(c.staged.is_empty());
    }

    /// Real-svn check for staging a directory together with its children:
    /// svn must accept redundant parent+child targets for add and commit.
    #[test]
    fn svn_accepts_dir_and_child_targets() {
        let Some(repo) = ts::TestRepo::new() else {
            return; // svn unavailable
        };
        // property change on a dir plus an added child, committed with
        // overlapping targets (dir first, then the child)
        ts::write_file(&repo.wc.join("src/new.rs"), "fn new() {}\n");
        repo.svn(&["add", "src/new.rs"]);
        repo.svn(&["propset", "svn:ignore", "*.tmp", "src"]);
        repo.svn(&["commit", "-m", "dir+child", "src", "src/new.rs"]);
        assert!(repo.svn(&["status"]).trim().is_empty());
        // deleted dir plus deleted child as overlapping targets
        repo.svn(&["rm", "docs"]);
        repo.svn(&["commit", "-m", "rm docs", "docs", "docs/readme.md"]);
        assert!(repo.svn(&["status"]).trim().is_empty());
        // an unversioned dir staged alone: `svn add dir` is recursive and
        // committing the dir target sweeps up its children
        ts::write_file(&repo.wc.join("newdir/f.txt"), "x\n");
        repo.svn(&["add", "newdir"]);
        let st = repo.svn(&["status"]);
        assert!(st.lines().any(|l| l.contains("newdir/f.txt")), "{st}");
        repo.svn(&["commit", "-m", "add newdir", "newdir"]);
        assert!(repo.svn(&["status"]).trim().is_empty());
    }
}

#[cfg(test)]
mod perf_tests {
    use super::*;
    use crate::test_support as ts;
    use crate::ui::style::Theme;
    use std::time::{Duration, Instant};

    fn comp() -> (StatusTreeComponent, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = crate::components::Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        (StatusTreeComponent::new(&ctx), q)
    }

    fn draw(comp: &StatusTreeComponent) {
        let _ = ts::render(120, 40, |f| {
            comp.draw(f, Rect::new(0, 0, 120, 40)).unwrap();
        });
    }

    /// Regression guard: the old `build_tree` was O(n^2) (linear child scan
    /// per insertion) and took ~9s *released* for 100k files in one
    /// directory. The new implementation is O(n).
    #[test]
    fn perf_update_100k_wide_stays_linear() {
        let (mut c, _q) = comp();
        let entries = ts::gen_status_entries(100_000, true);
        let t = Instant::now();
        c.update(entries);
        let el = t.elapsed();
        assert_eq!(c.visible.len(), 100_000);
        assert!(
            el < Duration::from_secs(10),
            "update(100k flat files) took {el:?}; O(n^2) regression?"
        );
    }

    #[test]
    fn perf_update_100k_deep() {
        let (mut c, _q) = comp();
        let entries = ts::gen_status_entries(100_000, false);
        let t = Instant::now();
        c.update(entries);
        let el = t.elapsed();
        assert!(
            el < Duration::from_secs(10),
            "update(100k nested files) took {el:?}"
        );
    }

    /// Drawing must only touch the visible window: rendering a 50k-entry
    /// tree 10 times must stay fast (virtualized rendering).
    #[test]
    fn perf_draw_large_tree_is_windowed() {
        let (mut c, _q) = comp();
        c.update(ts::gen_status_entries(50_000, true));
        let t = Instant::now();
        for _ in 0..10 {
            draw(&c);
        }
        let el = t.elapsed();
        assert!(
            el < Duration::from_secs(5),
            "10 draws of a 50k-entry tree took {el:?}; not windowed?"
        );
    }

    /// The per-directory staged counts must be cached across draws and only
    /// recomputed when the staged set / entries change.
    #[test]
    fn perf_counts_cache_reused_across_draws() {
        let (mut c, _q) = comp();
        c.update(ts::gen_status_entries(50_000, true));
        assert!(c.counts_dirty.get(), "initial update must mark dirty");
        draw(&c);
        assert!(!c.counts_dirty.get(), "first draw recomputes and clears");
        draw(&c);
        draw(&c);
        assert!(!c.counts_dirty.get(), "subsequent draws must reuse cache");
        // staging invalidates the cache
        c.toggle_stage_paths(&["file_000000.rs".to_string()]);
        assert!(c.counts_dirty.get());
        draw(&c);
        assert!(!c.counts_dirty.get());
    }

    /// Flatten (building the visible list) must stay linear.
    #[test]
    fn perf_flatten_100k() {
        let (mut c, _q) = comp();
        let entries = ts::gen_status_entries(100_000, true);
        c.update(entries);
        c.expanded.clear();
        let t = Instant::now();
        c.rebuild_visible();
        let el = t.elapsed();
        assert_eq!(c.visible.len(), 100_000);
        assert!(el < Duration::from_secs(5), "rebuild_visible took {el:?}");
    }

    /// Stage-all must not do a linear entry scan per path (the old handler
    /// was O(n^2): one `entry_for_path` lookup per changed file).
    #[test]
    fn perf_stage_all_100k() {
        let (mut c, _q) = comp();
        c.update(ts::gen_status_entries(100_000, true));
        let t = Instant::now();
        c.event(&Event::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('A'),
            crossterm::event::KeyModifiers::NONE,
        )))
        .unwrap();
        let el = t.elapsed();
        assert_eq!(c.staged_count(), 100_000);
        assert!(
            el < Duration::from_secs(5),
            "StageAll over 100k files took {el:?}; O(n^2) regression?"
        );
    }
}
