//! Parsers for `svn status` / `svn log` / `svn blame` / `svn diff` output.

use super::models::{
    BlameLine, DiffLine, DiffLineKind, LogEntry, ParsedDiff, StatusEntry, SvnInfo,
};

/// Parse the plain-text output of `svn info`.
///
/// Only the fields svnui needs are extracted: `URL`, `Relative URL`
/// (`^/trunk`, `^/branches/x`, ... — missing on very old svn versions),
/// `Revision` and `Working Copy Root Path`. `Repository URL:` must not be
/// mistaken for `URL:`.
pub fn parse_info(output: &str) -> SvnInfo {
    let mut url = String::new();
    let mut branch = String::new();
    let mut revision = 0;
    let mut wc_root = String::new();
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if let Some(v) = line.strip_prefix("URL:") {
            url = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Relative URL:") {
            // "^/branches/foo" → "branches/foo"
            branch = v
                .trim()
                .trim_start_matches('^')
                .trim_start_matches('/')
                .to_string();
        } else if let Some(v) = line.strip_prefix("Revision:") {
            revision = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("Working Copy Root Path:") {
            wc_root = v.trim().to_string();
        }
    }
    SvnInfo {
        url,
        branch,
        revision,
        wc_root,
    }
}

/// Parse the plain-text output of `svn status`.
///
/// Layout (verified against svn 1.14): seven single-character columns
/// (text, props, locked, copied+add, switched, unused, tree-conflict),
/// one separating space, then the path starting at byte 8 — the path may
/// itself begin with spaces, so no extra spaces may be skipped.
///
/// `root` is the working copy root: svn reports paths relative to it, so
/// `is_dir` must be probed against `root`, not the process CWD.
pub fn parse_status(output: &str, root: &std::path::Path) -> Vec<StatusEntry> {
    let mut entries = Vec::new();
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // Skip continuation lines (tree-conflict descriptions, move
        // sources): indented text starting with '>'
        if line.trim_start().starts_with('>') {
            continue;
        }
        let Some(path) = line.get(8..) else {
            continue;
        };
        if path.is_empty() {
            continue;
        }
        let bytes = line.as_bytes();
        let mut cols = [b' '; 7];
        cols.copy_from_slice(&bytes[..7]);
        let text = cols[0] as char;
        let props = cols[1] as char;
        let tree_conflict = cols[6] as char;
        if text == ' ' && props == ' ' && tree_conflict == ' ' {
            continue;
        }
        let is_dir = root.join(path).is_dir();
        entries.push(StatusEntry {
            status: text,
            props_status: props,
            tree_conflict,
            path: path.to_string(),
            is_dir,
        });
    }
    entries
}

/// Parse `svn log -v` output.
pub fn parse_log(output: &str) -> Vec<LogEntry> {
    // svn emits a hardcoded 72-dash separator between entries. Matching the
    // full line keeps a *message* line of dashes from being mistaken for an
    // entry boundary.
    const SEP: &str = "------------------------------------------------------------------------";
    let mut entries = Vec::new();
    let mut current: Option<LogEntry> = None;
    let mut message_lines: Vec<String> = Vec::new();
    let mut in_message = false;

    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');

        // Separator line: closes the current entry
        if line.starts_with(SEP) {
            if let Some(mut e) = current.take() {
                e.message = message_lines.join("\n").trim().to_string();
                entries.push(e);
            }
            message_lines.clear();
            in_message = false;
            continue;
        }

        if current.is_none() {
            // Header line: r123 | author | date | N lines
            if let Some(entry) = parse_log_header(line) {
                current = Some(entry);
                in_message = false;
            }
            continue;
        }

        // We have a current entry
        if line.is_empty() {
            if in_message {
                message_lines.push(String::new());
            }
            continue;
        }

        // The "Changed paths:" marker only appears between the header and
        // the message; once inside the message, such a line is content
        if !in_message && line == "Changed paths:" {
            continue;
        }

        // Changed path line: "   M /trunk/foo.txt" (3 leading spaces, '/')
        if !in_message && line.starts_with("   ") {
            let trimmed = line.trim_start();
            let mut chars = trimmed.chars();
            let (action, sep, rest) = (chars.next(), chars.next(), chars.as_str());
            // `rest` is taken via `chars.as_str()` so slicing stays on char
            // boundaries even when the line starts with CJK text
            if let (Some(action), Some(' ')) = (action, sep)
                && matches!(action, 'M' | 'A' | 'D' | 'R' | 'I' | 'X' | 'C' | '?')
                && rest.starts_with('/')
            {
                let path = rest.trim().trim_start_matches('/').to_string();
                if let Some(e) = current.as_mut() {
                    e.changed.push((action, path));
                }
                continue;
            }
        }

        // Anything else: message body
        in_message = true;
        message_lines.push(line.to_string());
    }

    // Flush trailing entry (in case output has no trailing separator)
    if let Some(mut e) = current.take() {
        e.message = message_lines.join("\n").trim().to_string();
        entries.push(e);
    }
    entries
}

fn parse_log_header(line: &str) -> Option<LogEntry> {
    // "r123 | author | 2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026) | 3 lines"
    let rest = line.strip_prefix('r')?;
    let mut parts = rest.splitn(4, " | ");
    let revision: u64 = parts.next()?.trim().parse().ok()?;
    let author = parts.next()?.trim().to_string();
    let date = parts.next()?.trim().to_string();
    let line_count_str = parts.next()?.trim();
    let line_count: u64 = line_count_str
        .split_whitespace()
        .next()?
        .parse()
        .unwrap_or(0);
    Some(LogEntry {
        revision,
        author,
        date,
        line_count,
        changed: Vec::new(),
        message: String::new(),
    })
}

/// Parse `svn blame` output.
///
/// Format (verified against svn 1.14): `%6s %10s %s`
/// revision right-justified in 6, author right-justified in 10, content.
/// Those are *minimum* widths: a long author (e.g. CJK, where 10 chars
/// exceed 10 bytes) or a 7-digit revision pushes the content right, so
/// parse by whitespace-separated tokens instead of fixed byte offsets.
pub fn parse_blame(output: &str) -> Vec<BlameLine> {
    let mut lines = Vec::new();
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        let (rev_str, rest) = next_token(line);
        if rev_str.is_empty() {
            continue;
        }
        let (author, rest) = next_token(rest);
        // exactly one separator space follows the author field; the file's
        // own leading indentation stays in the content
        let content = rest.strip_prefix(' ').unwrap_or(rest).to_string();
        let revision = if rev_str == "-" {
            None
        } else {
            rev_str.parse::<u64>().ok()
        };
        lines.push(BlameLine {
            revision,
            author: author.to_string(),
            content,
        });
    }
    lines
}

/// Split off the first space-delimited token, skipping leading spaces.
fn next_token(s: &str) -> (&str, &str) {
    let s = s.trim_start_matches(' ');
    match s.find(' ') {
        Some(end) => (&s[..end], &s[end..]),
        None => (s, ""),
    }
}

/// Parse `svn diff` output into line-numbered, colorizable lines.
pub fn parse_diff(output: &str) -> ParsedDiff {
    let mut lines = Vec::new();
    let mut old_line: Option<u64> = None;
    let mut new_line: Option<u64> = None;

    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if line.starts_with("Index:") || line.starts_with("===") {
            old_line = None;
            new_line = None;
            lines.push(DiffLine {
                old: None,
                new: None,
                kind: DiffLineKind::Header,
                content: line.to_string(),
            });
            continue;
        }
        // "--- "/"+++ " are file headers only right after an Index:/===
        // header line; inside a hunk they are content lines ("-- foo" is a
        // removed "- foo", "++ bar" an added "+ bar")
        let after_header = matches!(
            lines.last(),
            Some(l) if matches!(l.kind, DiffLineKind::Header | DiffLineKind::FileHeader)
        );
        if after_header && (line.starts_with("--- ") || line.starts_with("+++ ")) {
            lines.push(DiffLine {
                old: None,
                new: None,
                kind: DiffLineKind::FileHeader,
                content: line.to_string(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@") {
            let (o, n) = parse_hunk(rest);
            old_line = o;
            new_line = n;
            lines.push(DiffLine {
                old: None,
                new: None,
                kind: DiffLineKind::Hunk,
                content: line.to_string(),
            });
            continue;
        }
        if line.starts_with('\\') {
            lines.push(DiffLine {
                old: None,
                new: None,
                kind: DiffLineKind::Note,
                content: line.to_string(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix('+') {
            let n = new_line;
            new_line = new_line.map(|n| n + 1);
            lines.push(DiffLine {
                old: None,
                new: n,
                kind: DiffLineKind::Added,
                content: rest.to_string(),
            });
            continue;
        }
        if let Some(rest) = line.strip_prefix('-') {
            let o = old_line;
            old_line = old_line.map(|n| n + 1);
            lines.push(DiffLine {
                old: o,
                new: None,
                kind: DiffLineKind::Removed,
                content: rest.to_string(),
            });
            continue;
        }
        // Context line: leading space
        let rest = line.strip_prefix(' ').unwrap_or(line);
        let o = old_line;
        let n = new_line;
        old_line = old_line.map(|x| x + 1);
        new_line = new_line.map(|x| x + 1);
        lines.push(DiffLine {
            old: o,
            new: n,
            kind: DiffLineKind::Context,
            content: rest.to_string(),
        });
    }
    ParsedDiff { lines }
}

/// Parse "@@ -a,b +c,d @@" hunk header → (old_start, new_start).
fn parse_hunk(rest: &str) -> (Option<u64>, Option<u64>) {
    let mut old = None;
    let mut new = None;
    for part in rest.split_whitespace() {
        if let Some(r) = part.strip_prefix('-') {
            let nums: Vec<&str> = r.split(',').collect();
            old = nums[0].parse().ok();
        } else if let Some(r) = part.strip_prefix('+') {
            let nums: Vec<&str> = r.split(',').collect();
            new = nums[0].parse().ok();
        }
    }
    (old, new)
}

/// Build a simple line-numbered content view (for unversioned/new files).
pub fn parse_new_file_content(content: &str) -> ParsedDiff {
    let mut lines = Vec::new();
    for (n, raw) in (1_u64..).zip(content.lines()) {
        let line = raw.trim_end_matches('\r');
        lines.push(DiffLine {
            old: None,
            new: Some(n),
            kind: DiffLineKind::Added,
            content: line.to_string(),
        });
    }
    ParsedDiff { lines }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A root that matches nothing on disk (is_dir always false).
    fn no_root() -> &'static Path {
        Path::new("/svnui-test-no-such-dir")
    }

    #[test]
    fn test_parse_info() {
        let out = "\
Path: .
Working Copy Root Path: /home/user/wc
URL: https://svn.example.com/repos/proj/branches/feature-x
Relative URL: ^/branches/feature-x
Repository Root: https://svn.example.com/repos/proj
Repository UUID: 12345678-1234-1234-1234-123456789012
Revision: 1234
Node Kind: directory
Schedule: normal
Last Changed Rev: 1230
";
        let info = parse_info(out);
        assert_eq!(
            info.url,
            "https://svn.example.com/repos/proj/branches/feature-x"
        );
        assert_eq!(info.branch, "branches/feature-x");
        assert_eq!(info.revision, 1234);
        assert_eq!(info.wc_root, "/home/user/wc");
        assert_eq!(info.branch_label(), "branches/feature-x");
    }

    #[test]
    fn test_parse_info_trunk_and_missing_relative_url() {
        let out = "URL: file:///tmp/repo/trunk\nRelative URL: ^/trunk\nRevision: 2\n";
        let info = parse_info(out);
        assert_eq!(info.branch, "trunk");
        // old svn without Relative URL: branch stays empty, label = URL
        let old = parse_info("URL: file:///tmp/repo/trunk\nRevision: 7\n");
        assert_eq!(old.branch, "");
        assert_eq!(old.revision, 7);
        assert_eq!(old.branch_label(), "file:///tmp/repo/trunk");
    }

    #[test]
    fn test_parse_status() {
        let out = "M       Cargo.toml\n?       newfile.txt\nA  +    src/foo.rs\n";
        let entries = parse_status(out, no_root());
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].status, 'M');
        assert_eq!(entries[0].path, "Cargo.toml");
        assert_eq!(entries[1].status, '?');
        assert_eq!(entries[1].path, "newfile.txt");
        assert_eq!(entries[2].status, 'A');
        assert_eq!(entries[2].path, "src/foo.rs");
    }

    #[test]
    fn test_parse_status_missing_dir() {
        // path with missing dir: is_dir should be false without panicking
        let entries = parse_status("!       gone/dir\n", no_root());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, '!');
        assert!(!entries[0].is_dir);
    }

    #[test]
    fn test_parse_status_is_dir_relative_to_root() {
        let dir = std::env::temp_dir().join(format!("svnui-parse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("adir")).unwrap();
        let entries = parse_status("A       adir\nM       adir/f.txt\n", &dir);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_dir, "adir exists as a dir under root");
        assert!(!entries[1].is_dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_status_skips_tree_conflict_continuation() {
        // Real svn output for a tree conflict: the description continues on
        // an indented line starting with '>'
        let out = "A  +  C d\n      >   local dir edit, incoming dir delete upon update\n";
        let entries = parse_status(out, no_root());
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].status, 'A');
        assert_eq!(entries[0].tree_conflict, 'C');
        assert_eq!(entries[0].path, "d");
    }

    #[test]
    fn test_parse_status_path_with_leading_space() {
        // The path starts at byte 8 and may itself begin with spaces
        let out = format!("?       {}\n", " leading.txt");
        let entries = parse_status(&out, no_root());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, " leading.txt");
    }

    #[test]
    fn test_parse_log() {
        let out = "\
------------------------------------------------------------------------
r42 | alice | 2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026) | 2 lines
Changed paths:
   M /trunk/src/main.rs
   A /trunk/src/lib.rs

first line
second line

------------------------------------------------------------------------
r41 | bob | 2026-08-25 09:00:00 +0800 (Tue, 25 Aug 2026) | 1 line
Changed paths:
   D /trunk/old.txt

remove old
------------------------------------------------------------------------
";
        let entries = parse_log(out);
        assert_eq!(entries.len(), 2);
        let e = &entries[0];
        assert_eq!(e.revision, 42);
        assert_eq!(e.author, "alice");
        assert_eq!(e.changed.len(), 2);
        assert_eq!(e.changed[0], ('M', "trunk/src/main.rs".to_string()));
        assert_eq!(e.message, "first line\nsecond line");
        let e2 = &entries[1];
        assert_eq!(e2.revision, 41);
        assert_eq!(e2.changed[0], ('D', "trunk/old.txt".to_string()));
        assert_eq!(e2.message, "remove old");
    }

    #[test]
    fn test_parse_log_message_with_separator_like_lines() {
        // A message line of dashes (fewer than the 72-dash entry separator)
        // must stay part of the message
        let dashes40 = "-".repeat(40);
        let out = format!(
            "{sep}\nr7 | alice | 2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026) | 3 lines\n\nbefore\n{dashes40}\nafter\n{sep}\n",
            sep = "-".repeat(72),
        );
        let entries = parse_log(&out);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].message, format!("before\n{dashes40}\nafter"));
    }

    #[test]
    fn test_parse_log_message_mentions_changed_paths() {
        // A message line *containing* "Changed paths:" must not be eaten;
        // the real marker line is exactly "Changed paths:" before the message
        let out = "\
------------------------------------------------------------------------
r8 | alice | 2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026) | 2 lines
Changed paths:
   M /trunk/a.txt

Changed paths: this line is message content
second line
------------------------------------------------------------------------
";
        let entries = parse_log(out);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].changed.len(), 1);
        assert_eq!(
            entries[0].message,
            "Changed paths: this line is message content\nsecond line"
        );
    }

    #[test]
    fn test_parse_log_no_changed_paths() {
        let out = "\
------------------------------------------------------------------------
r5 | alice | 2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026) | 1 line
just a message
------------------------------------------------------------------------
";
        let entries = parse_log(out);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].revision, 5);
        assert_eq!(entries[0].message, "just a message");
    }

    #[test]
    fn test_parse_log_indented_cjk_message() {
        // An indented CJK first message line must not be mistaken for a
        // changed-path line (nor panic on a mid-char slice)
        let out = "\
------------------------------------------------------------------------
r7 | alice | 2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026) | 1 line
   中文说明
------------------------------------------------------------------------
";
        let entries = parse_log(out);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "中文说明");
        assert!(entries[0].changed.is_empty());
    }

    #[test]
    fn test_parse_blame() {
        let out = "     3    kenshin fn main() {\n     -          -     indented\n";
        let lines = parse_blame(out);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].revision, Some(3));
        assert_eq!(lines[0].author, "kenshin");
        assert_eq!(lines[0].content, "fn main() {");
        assert_eq!(lines[1].revision, None);
        assert_eq!(lines[1].author, "-");
        assert_eq!(lines[1].content, "    indented");
    }

    #[test]
    fn test_parse_blame_cjk_and_long_author() {
        // CJK author exceeds 10 bytes; 7-digit revision overflows its
        // 6-wide field — neither may break the column parsing
        let out = "     3 张三李四王五 fn main() {\n1234567 verylongauthorname x = 1\n";
        let lines = parse_blame(out);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].revision, Some(3));
        assert_eq!(lines[0].author, "张三李四王五");
        assert_eq!(lines[0].content, "fn main() {");
        assert_eq!(lines[1].revision, Some(1234567));
        assert_eq!(lines[1].author, "verylongauthorname");
        assert_eq!(lines[1].content, "x = 1");
    }

    #[test]
    fn test_parse_diff() {
        let out = "\
Index: Cargo.toml
===================================================================
--- Cargo.toml\t(revision 1)
+++ Cargo.toml\t(working copy)
@@ -1 +1,2 @@
 version = 1
+extra
";
        let d = parse_diff(out);
        assert_eq!(d.lines.len(), 7);
        assert_eq!(d.lines[0].kind, DiffLineKind::Header);
        assert_eq!(d.lines[4].kind, DiffLineKind::Hunk);
        assert_eq!(d.lines[5].kind, DiffLineKind::Context);
        assert_eq!(d.lines[5].new, Some(1));
        assert_eq!(d.lines[6].kind, DiffLineKind::Added);
        assert_eq!(d.lines[6].new, Some(2));
    }

    #[test]
    fn test_parse_diff_dash_prefixed_content_lines() {
        // "-- foo" / "++ bar" inside a hunk are content lines, not file
        // headers; misreading them shifts the line numbers afterwards
        let out = "\
Index: a.txt
===================================================================
--- a.txt\t(revision 1)
+++ a.txt\t(working copy)
@@ -1,2 +1,2 @@
-- foo
++ bar
 tail
";
        let d = parse_diff(out);
        assert_eq!(d.lines.len(), 8);
        assert_eq!(d.lines[2].kind, DiffLineKind::FileHeader);
        assert_eq!(d.lines[3].kind, DiffLineKind::FileHeader);
        assert_eq!(d.lines[4].kind, DiffLineKind::Hunk);
        assert_eq!(d.lines[5].kind, DiffLineKind::Removed);
        assert_eq!(d.lines[5].old, Some(1));
        assert_eq!(d.lines[5].content, "- foo");
        assert_eq!(d.lines[6].kind, DiffLineKind::Added);
        assert_eq!(d.lines[6].new, Some(1));
        assert_eq!(d.lines[6].content, "+ bar");
        assert_eq!(d.lines[7].kind, DiffLineKind::Context);
        assert_eq!(d.lines[7].old, Some(2));
        assert_eq!(d.lines[7].new, Some(2));
    }
}

#[cfg(test)]
mod perf_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn perf_parse_status_100k_lines() {
        let mut out = String::with_capacity(100_000 * 24);
        for i in 0..100_000 {
            out.push_str(&format!("M       src/file_{i:06}.rs\n"));
        }
        let t = Instant::now();
        let entries = parse_status(&out, std::path::Path::new("/svnui-test-no-such-dir"));
        let el = t.elapsed();
        assert_eq!(entries.len(), 100_000);
        assert!(
            el < Duration::from_secs(10),
            "parse_status(100k) took {el:?}"
        );
    }

    #[test]
    fn perf_parse_diff_50k_lines() {
        let mut out = String::with_capacity(50_000 * 24);
        out.push_str("Index: big.rs\n=== ===\n@@ -1 +1,50_000 @@\n");
        for i in 0..50_000 {
            out.push_str(&format!("+line {i}\n"));
        }
        let t = Instant::now();
        let d = parse_diff(&out);
        let el = t.elapsed();
        assert_eq!(d.lines.len(), 50_003);
        assert!(el < Duration::from_secs(10), "parse_diff(50k) took {el:?}");
    }
}
