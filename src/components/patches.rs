//! Patches tab: lists saved patch files (newest first) and offers preview
//! (in the diff popup), apply (`svn patch`) and delete actions.
//!
//! Listing is a tiny local directory read, so it is done synchronously —
//! no svn roundtrip. The directory is resolved once per component (see
//! `patch_dir`); `SVNUI_PATCH_DIR` overrides it (used by tests).

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::queue::{ConfirmAction, InternalEvent, Tab};
use crate::strings::{MSG, TITLE};
use crate::ui;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One patch file in the patch directory.
#[derive(Clone, Debug)]
pub struct PatchFile {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// Status-bar shortcut hints shown while the patches tab is active.
pub const HINTS: &str = "enter/p preview  a apply  d delete  P new patch  F5/R refresh";

pub struct PatchesComponent {
    ctx: Context,
    /// Directory the patch files live in
    dir: PathBuf,
    pub entries: Vec<PatchFile>,
    selection: usize,
    scroll: Cell<usize>,
}

impl PatchesComponent {
    pub fn new(ctx: &Context) -> Self {
        Self::with_dir(ctx, patch_dir())
    }

    /// A component listing an explicit directory (tests, future config).
    pub fn with_dir(ctx: &Context, dir: PathBuf) -> Self {
        let mut c = Self {
            ctx: ctx.clone(),
            dir,
            entries: Vec::new(),
            selection: 0,
            scroll: Cell::new(0),
        };
        c.refresh();
        c
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Point the tab at a different directory and reload it.
    pub fn set_dir(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.selection = 0;
        self.scroll.set(0);
        self.refresh();
    }

    /// Reload the list from disk (newest first).
    pub fn refresh(&mut self) {
        self.entries = list_patch_files(&self.dir);
        if self.selection >= self.entries.len() {
            self.selection = self.entries.len().saturating_sub(1);
        }
    }

    pub fn selection_entry(&self) -> Option<&PatchFile> {
        self.entries.get(self.selection)
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
        } else if key_match(k, KeyAction::Enter) || key_match(k, KeyAction::PreviewPatch) {
            if let Some(e) = self.selection_entry() {
                self.ctx
                    .queue
                    .push(InternalEvent::PreviewPatch(e.path.clone()));
            }
        } else if key_match(k, KeyAction::ApplyPatch) {
            if let Some(e) = self.selection_entry() {
                self.ctx
                    .queue
                    .push(InternalEvent::Confirm(ConfirmAction::ApplyPatch(
                        e.path.clone(),
                    )));
            }
        } else if key_match(k, KeyAction::DeletePatch) {
            if let Some(e) = self.selection_entry() {
                self.ctx
                    .queue
                    .push(InternalEvent::Confirm(ConfirmAction::DeletePatch(
                        e.path.clone(),
                    )));
            }
        } else if key_match(k, KeyAction::Refresh) {
            self.refresh();
        } else if key_match(k, KeyAction::Help) {
            self.ctx.queue.push(InternalEvent::OpenHelp);
        } else if key_match(k, KeyAction::Escape) || key_match(k, KeyAction::SwitchTabStatus) {
            self.ctx.queue.push(InternalEvent::SwitchTab(Tab::Status));
        } else if key_match(k, KeyAction::SwitchTabLog) {
            self.ctx.queue.push(InternalEvent::SwitchTab(Tab::Log));
        } else if key_match(k, KeyAction::SwitchTabPatches) {
            // already on the patches tab
        } else {
            return Ok(EventState::not_consumed());
        }
        Ok(EventState::consumed())
    }
}

impl DrawableComponent for PatchesComponent {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused))
            .title(format!("{} ({})", TITLE.patches, self.entries.len()));
        let inner = block.inner(area);
        f.render_widget(block, area);

        if self.entries.is_empty() {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(MSG.empty_patches, theme.dim))),
                inner,
            );
            return Ok(());
        }

        let view_h = inner.height as usize;
        // columns: name (flex) | size (right-aligned) | modified (UTC)
        let name_w = (inner.width as usize)
            .saturating_sub(8 + 2 + 16 + 2)
            .max(10);
        let lines: Vec<Line> = self
            .entries
            .iter()
            .map(|e| {
                let name = ui::truncate(&e.name, name_w);
                Line::from(vec![
                    Span::styled(format!("{name:<name_w$}"), theme.log_message),
                    Span::styled(format!("{:>8}", human_size(e.size)), theme.dim),
                    Span::raw("  "),
                    Span::styled(format_time(e.modified), theme.log_author),
                ])
            })
            .collect();

        let scroll = ui::scroll_follow(self.selection, self.scroll.get(), lines.len(), view_h);
        self.scroll.set(scroll);

        let highlights = vec![(self.selection, Style::default().bg(theme.selection_bg))];
        ui::render_lines(f, inner, &lines, scroll, &highlights);
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        self.event(ev)
    }
}

// ----- patch directory -----

/// Directory where patch files are stored.
///
/// `SVNUI_PATCH_DIR` overrides everything (used by tests). Otherwise the
/// platform data dir: `%APPDATA%\svnui\patches` on Windows,
/// `~/Library/Application Support/svnui/patches` on macOS,
/// `$XDG_DATA_HOME/svnui/patches` resp. `~/.local/share/svnui/patches` on
/// other unix. Falls back to a temp dir when nothing resolves. The
/// directory is created on demand when the first patch is saved.
pub fn patch_dir() -> PathBuf {
    resolve_patch_dir(std::env::var_os("SVNUI_PATCH_DIR").as_deref())
}

/// Pure dir resolution, testable without mutating the process env: a
/// non-empty `override_dir` wins, otherwise the platform dir (or temp).
fn resolve_patch_dir(override_dir: Option<&std::ffi::OsStr>) -> PathBuf {
    if let Some(dir) = override_dir
        && !dir.is_empty()
    {
        return PathBuf::from(dir);
    }
    platform_data_dir().unwrap_or_else(|| std::env::temp_dir().join("svnui").join("patches"))
}

#[cfg(target_os = "macos")]
fn platform_data_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("svnui")
            .join("patches"),
    )
}

#[cfg(target_os = "windows")]
fn platform_data_dir() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("svnui").join("patches"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_data_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join("svnui").join("patches"));
    }
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("svnui")
            .join("patches"),
    )
}

#[cfg(not(any(unix, target_os = "windows")))]
fn platform_data_dir() -> Option<PathBuf> {
    None
}

/// All regular files in the patch dir, newest modification first (the
/// timestamped names make the tiebreak deterministic).
fn list_patch_files(dir: &Path) -> Vec<PatchFile> {
    let mut out: Vec<PatchFile> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            out.push(PatchFile {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path(),
                size: meta.len(),
                modified: meta.modified().ok(),
            });
        }
    }
    out.sort_by(newer_first);
    out
}

/// List order: newest modification first; same-second saves (timestamped
/// names) fall back to name descending, keeping the order deterministic.
fn newer_first(a: &PatchFile, b: &PatchFile) -> std::cmp::Ordering {
    b.modified
        .cmp(&a.modified)
        .then_with(|| b.name.cmp(&a.name))
}

// ----- naming & formatting -----

/// `patch-YYYYMMDD-HHMMSS.patch` (UTC).
pub fn patch_file_name(t: SystemTime) -> String {
    let (y, mo, d, hh, mi, ss) = utc_ymd_hms(t);
    format!("patch-{y:04}{mo:02}{d:02}-{hh:02}{mi:02}{ss:02}.patch")
}

/// A patch path in `dir` that does not exist yet: same-second saves get a
/// `-2`, `-3`, ... suffix instead of silently overwriting each other.
pub fn fresh_patch_path(dir: &Path, t: SystemTime) -> PathBuf {
    let base = patch_file_name(t);
    let path = dir.join(&base);
    if !path.exists() {
        return path;
    }
    let stem = base.trim_end_matches(".patch");
    for i in 2u32.. {
        let candidate = dir.join(format!("{stem}-{i}.patch"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // the u32 range above is exhausted only after ~4 billion same-second
    // patches — fall back to a nanosecond-suffixed name instead of panicking
    let nanos = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    dir.join(format!("{stem}-n{nanos}.patch"))
}

/// Human readable file size ("512 B", "1.5 KiB", ...).
pub(crate) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

/// `YYYY-MM-DD HH:MM` (UTC) for the list's modification column.
fn format_time(t: Option<SystemTime>) -> String {
    match t {
        Some(t) => {
            let (y, mo, d, hh, mi, _) = utc_ymd_hms(t);
            format!("{y:04}-{mo:02}-{d:02} {hh:02}:{mi:02}")
        }
        None => "?".to_string(),
    }
}

/// UTC calendar fields of a `SystemTime` (hand-rolled civil-from-days;
/// keeps the dependency list free of chrono/time).
fn utc_ymd_hms(t: SystemTime) -> (i64, u64, u64, u64, u64, u64) {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days algorithm
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;
    use crate::ui::style::Theme;
    use crossterm::event::KeyCode;

    fn ctx() -> (Context, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        (
            Context {
                queue: q.clone(),
                theme: Theme::default(),
            },
            q,
        )
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("svnui-patches-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_patch(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn patch(name: &str, size: u64, modified_secs: u64) -> PatchFile {
        PatchFile {
            name: name.to_string(),
            path: PathBuf::from(name),
            size,
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(modified_secs)),
        }
    }

    #[test]
    fn resolve_patch_dir_honors_override() {
        // pure function: no env mutation needed
        let dir = PathBuf::from("/tmp/somewhere");
        assert_eq!(resolve_patch_dir(Some(dir.as_os_str())), dir);
        // an empty override is ignored
        let fallback = resolve_patch_dir(Some(std::ffi::OsStr::new("")));
        assert!(
            fallback.ends_with(Path::new("svnui").join("patches")),
            "{fallback:?}"
        );
        // no override: the platform dir resolves
        let resolved = resolve_patch_dir(None);
        assert!(
            resolved.ends_with(Path::new("svnui").join("patches")),
            "{resolved:?}"
        );
    }

    #[test]
    fn newer_first_sorts_mtime_desc_then_name_desc() {
        use std::cmp::Ordering;
        let new = patch("new.patch", 5, 200);
        let mid = patch("mid.patch", 1, 150);
        let old = patch("old.patch", 3, 100);
        // different mtimes: newest first, regardless of name
        assert_eq!(newer_first(&new, &mid), Ordering::Less);
        assert_eq!(newer_first(&old, &mid), Ordering::Greater);
        assert_eq!(newer_first(&old, &new), Ordering::Greater);
        // equal mtime: name descending breaks the tie
        let a = patch("a.patch", 1, 100);
        let b = patch("b.patch", 1, 100);
        assert_eq!(newer_first(&a, &b), Ordering::Greater);
        assert_eq!(newer_first(&b, &a), Ordering::Less);
        // identical entries are equal
        assert_eq!(newer_first(&a, &patch("a.patch", 1, 100)), Ordering::Equal);
        // mtime dominates the name tiebreak
        assert_eq!(newer_first(&b, &new), Ordering::Greater);
        // entries without a timestamp sort last
        let mut unknown = patch("z.patch", 1, 100);
        unknown.modified = None;
        assert_eq!(newer_first(&unknown, &old), Ordering::Greater);
    }

    #[test]
    fn refresh_lists_newest_first() {
        let (c, _q) = ctx();
        let dir = temp_dir("list");
        write_patch(&dir, "a.patch", "a");
        write_patch(&dir, "b.patch", "bb");
        write_patch(&dir, "c.patch", "ccc");
        // a subdirectory is not a patch file
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        // pin the mtimes (filesystem timestamps are too coarse to rely on
        // the write order): b newest, then a, then c
        let set_mtime = |name: &str, secs: u64| {
            let f = std::fs::File::options()
                .write(true)
                .open(dir.join(name))
                .unwrap();
            f.set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs))
                .unwrap();
        };
        set_mtime("a.patch", 200);
        set_mtime("b.patch", 300);
        set_mtime("c.patch", 100);
        let mut comp = PatchesComponent::with_dir(&c, dir.clone());
        // the directory read itself returns the entries newest first
        let names: Vec<&str> = comp.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["b.patch", "a.patch", "c.patch"], "{names:?}");
        // same mtime: the name-descending tiebreak decides
        set_mtime("c.patch", 300);
        comp.refresh();
        let names: Vec<&str> = comp.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["c.patch", "b.patch", "a.patch"], "{names:?}");
        // refresh drops deleted files and clamps the selection
        comp.selection = 2;
        std::fs::remove_file(dir.join("b.patch")).unwrap();
        comp.refresh();
        let names: Vec<&str> = comp.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["c.patch", "a.patch"], "{names:?}");
        assert_eq!(comp.selection, 1);
        assert_eq!(comp.selection_entry().unwrap().name, "a.patch");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keys_push_events() {
        let (c, q) = ctx();
        let dir = temp_dir("keys");
        write_patch(&dir, "a.patch", "x");
        write_patch(&dir, "b.patch", "yy");
        let mut comp = PatchesComponent::with_dir(&c, dir.clone());
        assert_eq!(comp.entries.len(), 2);

        // navigation (both files have ~the same mtime; order by name desc)
        let first = comp.selection_entry().unwrap().name.clone();
        comp.event(&ts::key(KeyCode::Char('j'))).unwrap();
        let second = comp.selection_entry().unwrap().name.clone();
        assert_ne!(first, second);
        comp.event(&ts::key(KeyCode::Char('k'))).unwrap();
        comp.event(&ts::key(KeyCode::End)).unwrap();
        comp.event(&ts::key(KeyCode::Home)).unwrap();
        comp.event(&ts::key(KeyCode::PageDown)).unwrap();
        comp.event(&ts::key(KeyCode::PageUp)).unwrap();

        // preview via Enter and 'p'
        comp.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::PreviewPatch(_))));
        comp.event(&ts::key(KeyCode::Char('p'))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::PreviewPatch(_))));

        // apply / delete ask for confirmation
        comp.event(&ts::key(KeyCode::Char('a'))).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Confirm(ConfirmAction::ApplyPatch(_)))
        ));
        comp.event(&ts::key(KeyCode::Char('d'))).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Confirm(ConfirmAction::DeletePatch(_)))
        ));

        // refresh is local (no queue event), help and tab switches queue
        comp.event(&ts::key(KeyCode::F(5))).unwrap();
        comp.event(&ts::key(KeyCode::Char('R'))).unwrap();
        comp.event(&ts::key(KeyCode::Char('?'))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::OpenHelp)));
        comp.event(&ts::key(KeyCode::Char('1'))).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::SwitchTab(Tab::Status))
        ));
        comp.event(&ts::key(KeyCode::Char('2'))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::SwitchTab(Tab::Log))));
        comp.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::SwitchTab(Tab::Status))
        ));
        comp.event(&ts::key(KeyCode::Char('3'))).unwrap(); // already here, consumed
        // q is not consumed so the app can quit
        assert!(!comp.event(&ts::key(KeyCode::Char('q'))).unwrap().consumed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_dir_shows_hint_and_keys_are_noops() {
        let (c, q) = ctx();
        let dir = temp_dir("empty");
        let mut comp = PatchesComponent::with_dir(&c, dir.clone());
        assert!(comp.entries.is_empty());
        // actions on an empty list push nothing
        comp.event(&ts::key(KeyCode::Enter)).unwrap();
        comp.event(&ts::key(KeyCode::Char('a'))).unwrap();
        comp.event(&ts::key(KeyCode::Char('d'))).unwrap();
        comp.event(&ts::key(KeyCode::Char('j'))).unwrap();
        assert!(q.pop().is_none());
        // the hint explains how to create a patch
        let t = ts::render(100, 8, |f| {
            comp.draw(f, Rect::new(0, 0, 100, 8)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("No patches yet"), "{s}");
        assert!(s.contains("press P to save"), "{s}");
        assert!(s.contains("Patches (0)"), "{s}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn draw_lists_name_size_and_time() {
        let (c, _q) = ctx();
        let mut comp = PatchesComponent::with_dir(&c, temp_dir("draw"));
        comp.entries = vec![
            patch("patch-20260102-000000.patch", 2048, 1_700_000_000),
            patch("patch-20260101-000000.patch", 512, 1_600_000_000),
        ];
        let t = ts::render(100, 8, |f| {
            comp.draw(f, Rect::new(0, 0, 100, 8)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("Patches (2)"), "{s}");
        assert!(s.contains("patch-20260102-000000.patch"), "{s}");
        assert!(s.contains("2.0 KiB"), "{s}");
        assert!(s.contains("512 B"), "{s}");
        // newest first: 2023-11-14 22:13 UTC before 2020-09-13 12:26 UTC
        assert!(s.contains("2023-11-14 22:13"), "{s}");
        assert!(s.contains("2020-09-13 12:26"), "{s}");
        let newest = s.find("patch-20260102").unwrap();
        let oldest = s.find("patch-20260101").unwrap();
        assert!(newest < oldest, "{s}");
        let _ = std::fs::remove_dir_all(comp.dir());
    }

    #[test]
    fn scroll_window_follows_selection() {
        let (c, _q) = ctx();
        let mut comp = PatchesComponent::with_dir(&c, temp_dir("scroll"));
        comp.entries = (0..30)
            .map(|i| patch(&format!("p{i:02}.patch"), 1, 100 + i as u64))
            .collect();
        // inner height of a 60x6 block is 4 rows: jumping to the end must
        // scroll the window so the selection stays visible
        comp.event(&ts::key(KeyCode::End)).unwrap();
        assert_eq!(comp.selection, 29);
        let t = ts::render(60, 6, |f| {
            comp.draw(f, Rect::new(0, 0, 60, 6)).unwrap();
        });
        assert_eq!(comp.scroll.get(), 26);
        let s = ts::dump(&t);
        assert!(s.contains("p29.patch"), "{s}");
        assert!(s.contains("p26.patch"), "{s}");
        assert!(!s.contains("p25.patch"), "scrolled-out row drawn: {s}");
        // moving back up scrolls the window up again
        comp.event(&ts::key(KeyCode::Home)).unwrap();
        ts::render(60, 6, |f| {
            comp.draw(f, Rect::new(0, 0, 60, 6)).unwrap();
        });
        assert_eq!(comp.scroll.get(), 0);
        let _ = std::fs::remove_dir_all(comp.dir());
    }

    #[test]
    fn naming_and_size_helpers() {
        // 2026-01-02 03:04:05 UTC = 1767323045
        let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_767_323_045);
        assert_eq!(patch_file_name(t), "patch-20260102-030405.patch");
        // pre-epoch times clamp to the epoch instead of panicking
        let epoch = SystemTime::UNIX_EPOCH;
        assert_eq!(patch_file_name(epoch), "patch-19700101-000000.patch");

        // same-second saves do not overwrite each other
        let dir = temp_dir("fresh");
        let p1 = fresh_patch_path(&dir, epoch);
        assert_eq!(p1.file_name().unwrap(), "patch-19700101-000000.patch");
        std::fs::write(&p1, "x").unwrap();
        let p2 = fresh_patch_path(&dir, epoch);
        assert_eq!(p2.file_name().unwrap(), "patch-19700101-000000-2.patch");
        std::fs::write(&p2, "x").unwrap();
        let p3 = fresh_patch_path(&dir, epoch);
        assert_eq!(p3.file_name().unwrap(), "patch-19700101-000000-3.patch");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KiB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_time(None), "?");
    }
}
