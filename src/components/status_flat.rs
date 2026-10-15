//! Flat status view helpers: compact display of only M/A/D files.
//!
//! The status tab has a flat mode (key `f`) that lists modified, added and
//! deleted files instead of the directory tree. Two display rules keep the
//! rows short while still showing where a file lives:
//!
//! 1. intermediate directory segments are abbreviated to their first
//!    character (`include/spdlog/fmt/fmt.h` -> `i/s/f/fmt.h`); hidden
//!    (dot) directories and one-character segments keep their name, and an
//!    abbreviation that collides with another row falls back to the full
//!    path so rows stay unambiguous;
//! 2. at render time a display that is still too wide for the pane gets
//!    its middle segments elided (`a/b/c/d/file.rs` -> `a/b/.../file.rs`),
//!    always keeping the file name.

use std::collections::HashMap;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Marker inserted between path segments when they are elided.
pub const ELISION: &str = "...";

/// Status codes listed in the flat view: modified, added, deleted.
///
/// Directories are excluded — the flat view lists files only (a deleted
/// directory still shows up through its `D` children).
pub fn is_flat_candidate(code: char, is_dir: bool) -> bool {
    !is_dir && matches!(code, 'M' | 'A' | 'D')
}

/// Abbreviate every intermediate directory segment to its first character:
/// `adir/bdir/cdir/file` -> `a/b/c/file`. The file name and dot
/// directories (`.github/...`) are kept whole; one-character segments are
/// already minimal.
pub fn abbreviate_dirs(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() <= 1 {
        return path.to_string();
    }
    let mut out = String::with_capacity(path.len());
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push('/');
        }
        let is_file = i + 1 == parts.len();
        if is_file || part.starts_with('.') || part.chars().count() <= 1 {
            out.push_str(part);
        } else if let Some(first) = part.chars().next() {
            out.push(first);
        }
    }
    out
}

/// Flat display names for `paths`, in the same order. A path whose
/// abbreviation collides with another entry keeps its full path, so two
/// different files never render as the same row.
pub fn display_names<'a>(paths: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let paths: Vec<&str> = paths.into_iter().collect();
    let short: Vec<String> = paths.iter().map(|p| abbreviate_dirs(p)).collect();
    let mut counts: HashMap<&str, usize> = HashMap::with_capacity(short.len());
    for s in &short {
        *counts.entry(s.as_str()).or_default() += 1;
    }
    let mut out = Vec::with_capacity(short.len());
    for (s, full) in short.iter().zip(paths) {
        if counts[s.as_str()] == 1 {
            out.push(s.clone());
        } else {
            out.push(full.to_string());
        }
    }
    out
}

/// Fit `display` into `max_width` columns by eliding middle segments:
/// the first segments that fit and the last segment are kept,
/// `a/b/c/d/file.rs` -> `a/b/.../file.rs`. The candidates are tried in
/// order of decreasing context, and the file name is never dropped for a
/// directory segment:
///
/// 1. the longest leading run that still fits next to `.../<file>`;
/// 2. `.../<file>` alone;
/// 3. the bare file name (a complete name beats a marked-up partial one);
/// 4. a clipped name — for a two-segment path the leading directory is
///    kept so same-named files stay distinguishable.
///
/// All width checks are display-width aware (CJK counts as two columns).
pub fn elide_middle(display: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(display) <= max_width {
        return display.to_string();
    }
    let parts: Vec<&str> = display.split('/').collect();
    if parts.len() <= 1 {
        return truncate_width(display, max_width);
    }
    let last = parts[parts.len() - 1];
    let mut head = String::new();
    let mut best: Option<String> = None;
    for part in &parts[..parts.len() - 1] {
        if !head.is_empty() {
            head.push('/');
        }
        head.push_str(part);
        let candidate = format!("{head}/{ELISION}/{last}");
        if UnicodeWidthStr::width(candidate.as_str()) <= max_width {
            best = Some(candidate);
        } else {
            break;
        }
    }
    if let Some(elided) = best {
        return elided;
    }
    let tail = format!("{ELISION}/{last}");
    if UnicodeWidthStr::width(tail.as_str()) <= max_width {
        return tail;
    }
    if UnicodeWidthStr::width(last) <= max_width {
        return last.to_string();
    }
    if parts.len() == 2 {
        truncate_width(display, max_width)
    } else {
        truncate_width(last, max_width)
    }
}

/// Clip `s` to `max_width` display columns, ending with `…` when content
/// had to be dropped (CJK chars count as two columns).
fn truncate_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max_width {
        return s.to_string();
    }
    let mut out = String::new();
    let mut width = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > max_width.saturating_sub(1) {
            break;
        }
        out.push(ch);
        width += cw;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_candidates_are_mad_files_only() {
        for code in ['M', 'A', 'D'] {
            assert!(is_flat_candidate(code, false), "{code}");
        }
        for code in ['?', '!', 'C', 'R', '~', 'I', ' '] {
            assert!(!is_flat_candidate(code, false), "{code}");
        }
        // a directory with an A/D status stays out of the file list
        assert!(!is_flat_candidate('D', true));
        assert!(!is_flat_candidate('A', true));
    }

    #[test]
    fn abbreviate_dirs_keeps_file_and_dot_dirs() {
        assert_eq!(abbreviate_dirs("adir/bdir/cdir/file"), "a/b/c/file");
        assert_eq!(abbreviate_dirs("include/spdlog/fmt/fmt.h"), "i/s/f/fmt.h");
        // single-segment paths and already-short segments are untouched
        assert_eq!(abbreviate_dirs("Cargo.toml"), "Cargo.toml");
        assert_eq!(abbreviate_dirs("a/b/c/file"), "a/b/c/file");
        // hidden directories keep their whole name ('.' would be useless)
        assert_eq!(
            abbreviate_dirs(".github/workflows/ci.yml"),
            ".github/w/ci.yml"
        );
        // CJK directory names abbreviate per char, not per byte
        assert_eq!(abbreviate_dirs("源码/核心/主.rs"), "源/核/主.rs");
    }

    #[test]
    fn display_names_fall_back_on_collision() {
        let names = display_names([
            "src/app/main.rs",
            "src/api/main.rs",
            "src/lib/util.rs",
            "Cargo.toml",
        ]);
        // src/app and src/api both abbreviate to s/a/main.rs -> full paths
        assert_eq!(names[0], "src/app/main.rs");
        assert_eq!(names[1], "src/api/main.rs");
        // unique abbreviations are kept
        assert_eq!(names[2], "s/l/util.rs");
        assert_eq!(names[3], "Cargo.toml");
    }

    #[test]
    fn elide_middle_keeps_the_file_name() {
        let long = "alpha/beta/gamma/delta/epsilon/file.rs";
        // fits: untouched
        assert_eq!(elide_middle(long, 100), long);
        // keeps the leading segments that fit, then elides, keeps the file
        assert_eq!(
            elide_middle(long, UnicodeWidthStr::width("alpha/beta/.../file.rs")),
            "alpha/beta/.../file.rs"
        );
        assert_eq!(elide_middle(long, 20), "alpha/.../file.rs");
        // nothing fits before the file: leading segments collapse to ...
        assert_eq!(elide_middle(long, 11), ".../file.rs");
        // even the elision prefix does not fit: show the whole file name
        assert_eq!(elide_middle(long, 8), "file.rs");
        // short paths get a plain clip instead of a pointless .../ prefix
        assert_eq!(elide_middle("a/longfile.rs", 5), "a/lo…");
    }

    #[test]
    fn elide_middle_counts_display_width() {
        // CJK file name (3 wide chars + ".rs" = 9 columns): the complete
        // name fits and is preferred over an elided alternative
        assert_eq!(elide_middle("a/b/文件名.rs", 9), "文件名.rs");
        // too narrow for the whole name: width-aware clip, 2 CJK chars + …
        assert_eq!(elide_middle("a/b/文件名.rs", 6), "文件…");
        assert_eq!(elide_middle("a/b/文件名.rs", 5), "文件…");
        // zero width never panics
        assert_eq!(elide_middle("a/b/c", 0), "");
        assert_eq!(elide_middle("a/b/c", 1), "c");
        // single segment (no slash): plain width-aware clip
        assert_eq!(elide_middle("文件名.rs", 4), "文…");
    }
}
