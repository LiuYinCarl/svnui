//! Parsers for `svn status` / `svn log` / `svn blame` / `svn diff` output.

use super::models::{
    BlameLine, DiffLine, DiffLineKind, LogEntry, ParsedDiff, StatusEntry, SvnInfo,
};

/// Parse `svn --version --quiet` output ("1.14.5" or "1.14.5 (r1876290)")
/// into (major, minor, patch).
pub fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let text = text.trim();
    let mut parts = text.split(['.', ' ', '(']);
    let major: u32 = parts.next()?.trim().parse().ok()?;
    let minor: u32 = parts.next()?.trim().parse().ok()?;
    let patch: u32 = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
    Some((major, minor, patch))
}

/// Lexicographic version comparison: is `v` at least `min`?
pub fn version_at_least(v: (u32, u32, u32), min: (u32, u32, u32)) -> bool {
    v >= min
}

/// Parse the plain-text output of `svn info`.
///
/// `URL`, `Relative URL` (`^/trunk`, `^/branches/x`, ... — missing on very
/// old svn versions), `Revision`, `Working Copy Root Path`, `Repository
/// Root`, `Repository UUID` and the `Last Changed *` triple are extracted.
/// `Repository URL:` must not be mistaken for `URL:`.
pub fn parse_info(output: &str) -> SvnInfo {
    let mut url = String::new();
    let mut branch = String::new();
    let mut revision = 0;
    let mut wc_root = String::new();
    let mut repo_root = String::new();
    let mut uuid = String::new();
    let mut last_author = String::new();
    let mut last_rev = 0;
    let mut last_date = String::new();
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
        } else if let Some(v) = line.strip_prefix("Repository Root:") {
            repo_root = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Repository UUID:") {
            uuid = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Last Changed Author:") {
            last_author = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Last Changed Rev:") {
            last_rev = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("Last Changed Date:") {
            last_date = v.trim().to_string();
        }
    }
    SvnInfo {
        url,
        branch,
        revision,
        wc_root,
        repo_root,
        uuid,
        last_author,
        last_rev,
        last_date,
    }
}

/// Parse the plain-text output of `svn status`.
///
/// Layout (verified against svn 1.14): seven single-character columns
/// (text, props, locked, copied+add, switched, unused, tree-conflict),
/// one separating space, then the path starting at byte 8 — the path may
/// itself begin with spaces, so no extra spaces may be skipped.
///
/// Known plain-text parsing limitations: non-UTF-8 paths become lossy
/// (the output was decoded with `String::from_utf8_lossy`), and a
/// filename containing a newline misaligns the affected entry.
///
/// `root` is the working copy root: svn reports paths relative to it, so
/// `is_dir` must be probed against `root`, not the process CWD.
pub fn parse_status(output: &str, root: &std::path::Path) -> Vec<StatusEntry> {
    let mut entries = Vec::new();
    let mut in_conflict_summary = false;
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // Conflict trailer (printed last when the wc has conflicts):
        // "Summary of conflicts:" plus its indented detail lines
        // ("  Text conflicts: 1", ...). Skip both explicitly — the first
        // line would otherwise yield a phantom 'S' entry with path
        // "of conflicts:", and the detail lines must not rely on the
        // blank-column check below happening to reject them.
        if line == "Summary of conflicts:" {
            in_conflict_summary = true;
            continue;
        }
        if in_conflict_summary && line.starts_with("  ") {
            continue;
        }
        in_conflict_summary = false;
        // Skip continuation lines (tree-conflict descriptions, move
        // sources): svn prints them as exactly six spaces + '>'. Anchor
        // on that — a looser check would swallow entries whose *path*
        // (starting at byte 8, possibly with leading spaces) begins
        // with '>'.
        if line.starts_with("      >") {
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
///
/// The header's line count — not the separator — bounds the message
/// body: a message line of exactly 72 dashes, or one that looks like a
/// header ("r99 | fake | ..."), is content. The entry still ends at an
/// exact 72-dash separator line after the message has been consumed.
pub fn parse_log(output: &str) -> Vec<LogEntry> {
    // svn emits exactly 72 dashes as the separator between entries.
    // Require an exact match: a *message* line of dashes (shorter *or*
    // longer than 72) is content, not an entry boundary.
    const SEP: &str = "------------------------------------------------------------------------";
    let mut entries = Vec::new();
    let mut current: Option<LogEntry> = None;
    let mut message_lines: Vec<String> = Vec::new();
    // Per-entry phase: changed-paths block → message body (the header's
    // line count lines, whatever they look like) → done (wait for the
    // separator)
    enum Phase {
        Paths,
        Message(u64),
        Done,
    }
    let mut phase = Phase::Paths;

    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');

        // Separator line: closes the current entry — but not inside the
        // message body, where a 72-dash line is content
        if line == SEP && !matches!(phase, Phase::Message(_)) {
            if let Some(mut e) = current.take() {
                e.message = message_lines.join("\n").trim().to_string();
                entries.push(e);
            }
            message_lines.clear();
            phase = Phase::Paths;
            continue;
        }

        let Some(entry) = current.as_mut() else {
            // Header line: r123 | author | date | N lines
            if let Some(e) = parse_log_header(line) {
                current = Some(e);
            }
            continue;
        };

        match phase {
            // Message body: consume exactly line_count lines (dashes,
            // header-looking lines, blank lines — all content)
            Phase::Message(rem) => {
                message_lines.push(line.to_string());
                phase = if rem > 1 {
                    Phase::Message(rem - 1)
                } else {
                    Phase::Done
                };
            }
            Phase::Done => {}
            // Changed-paths phase: "Changed paths:" block, then a blank
            // line, then the message. svn always prints the blank line,
            // but tolerate its absence: a non-path line starts the
            // message directly.
            Phase::Paths => {
                if line.is_empty() {
                    phase = if entry.line_count > 0 {
                        Phase::Message(entry.line_count)
                    } else {
                        Phase::Done
                    };
                    continue;
                }
                if line == "Changed paths:" {
                    continue;
                }

                // Changed path: "   M /trunk/foo.txt" (3 spaces, then '/')
                if line.starts_with("   ") {
                    let trimmed = line.trim_start();
                    let mut chars = trimmed.chars();
                    let (action, sep, rest) = (chars.next(), chars.next(), chars.as_str());
                    // `rest` is taken via `chars.as_str()` so slicing stays
                    // on char boundaries even with leading CJK text
                    if let (Some(action), Some(' ')) = (action, sep)
                        && matches!(action, 'M' | 'A' | 'D' | 'R' | 'I' | 'X' | 'C' | '?')
                        && rest.starts_with('/')
                    {
                        let path = rest.trim().trim_start_matches('/').to_string();
                        entry.changed.push((action, path));
                        continue;
                    }
                }

                // Anything else: first message line (the blank separator
                // line between changed paths and message is missing)
                message_lines.push(line.to_string());
                phase = if entry.line_count > 1 {
                    Phase::Message(entry.line_count - 1)
                } else {
                    Phase::Done
                };
            }
        }
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
    // svn does not sanitize the author, which may itself contain " | " —
    // so anchor the revision at the left and the line count + date at
    // the right; whatever remains in the middle is the author.
    let rest = line.strip_prefix('r')?;
    let (rev_str, rest) = rest.split_once(" | ")?;
    let revision: u64 = rev_str.trim().parse().ok()?;
    let (middle, line_count_str) = rest.rsplit_once(" | ")?;
    let (author, date) = middle.rsplit_once(" | ")?;
    let line_count: u64 = line_count_str
        .split_whitespace()
        .next()?
        .parse()
        .unwrap_or(0);
    Some(LogEntry {
        revision,
        author: author.trim().to_string(),
        date: date.trim().to_string(),
        line_count,
        changed: Vec::new(),
        message: String::new(),
    })
}

/// Parse `svn blame --xml` output into (line number, author) pairs (None
/// author for uncommitted lines, which have no `<commit>` block). The XML
/// is the only blame output where authors survive intact: the plain
/// format truncates the author field to 10 bytes and cannot express names
/// containing spaces. Only the flat, fixed structure is parsed — no XML
/// library.
///
/// ```xml
/// <entry
///    line-number="1">
/// <commit
///    revision="1">
/// <author>Gabi Melman</author>
/// ...
/// ```
pub fn parse_blame_xml(xml: &str) -> Vec<(u64, Option<String>)> {
    let mut entries = Vec::new();
    for chunk in xml.split("<entry").skip(1) {
        let line_number = chunk
            .split_once("line-number=\"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .and_then(|(n, _)| n.parse().ok())
            .unwrap_or(0);
        let author = chunk
            .split_once("<author>")
            .and_then(|(_, rest)| rest.split_once("</author>"))
            .map(|(name, _)| xml_unescape(name));
        entries.push((line_number, author));
    }
    entries
}

/// Replace the five predefined XML entities.
fn xml_unescape(s: &str) -> String {
    // &amp; first: it introduces the other entities' '&'
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Overlay exact authors from `parse_blame_xml` onto text-parsed blame
/// lines, matched by line number (the text side's position + 1). A None
/// entry (uncommitted line) keeps the text-side author ("-"). When the
/// two sides disagree — different line counts, or a line number outside
/// the text output — the merge is abandoned silently and the text-parsed
/// authors are kept (the two blame runs must have seen different
/// content, so positional pairing would misattribute authors).
pub fn merge_blame_authors(lines: &mut [BlameLine], authors: &[(u64, Option<String>)]) {
    if authors.len() != lines.len()
        || authors
            .iter()
            .any(|(n, _)| *n == 0 || *n > lines.len() as u64)
    {
        return;
    }
    for (n, author) in authors {
        if let Some(a) = author {
            lines[(*n - 1) as usize].author = a.clone();
        }
    }
}

/// Parse `svn blame` plain-text output (raw bytes).
///
/// Format (verified against svn 1.14): `%6s %10s %s` — revision
/// right-justified in *min* 6 bytes (7-digit revisions overflow), one
/// space, author right-justified in *exactly* 10 bytes (longer names are
/// byte-truncated, possibly mid-CJK-char), one space, then the content.
///
/// Fixed byte columns are required: whitespace-token parsing mistakes an
/// author containing a space ("Gabi Melma") for content. Fields are
/// lossy-decoded individually so a mid-char truncation garbles only the
/// author (overlaid by `parse_blame_xml` upstream), never the content.
pub fn parse_blame(output: &[u8]) -> Vec<BlameLine> {
    let mut lines = Vec::new();
    for raw in output.split(|&b| b == b'\n') {
        let line = raw.strip_suffix(b"\r").unwrap_or(raw);
        // revision token: skip its leading padding, then read to the space
        let Some(start) = line.iter().position(|&b| b != b' ') else {
            continue;
        };
        let end = line[start..]
            .iter()
            .position(|&b| b == b' ')
            .map(|i| start + i)
            .unwrap_or(line.len());
        let rev_str = &line[start..end];
        // author field: exactly 10 bytes after one separator space
        let author = String::from_utf8_lossy(line.get(end + 1..end + 11).unwrap_or(&[]))
            .trim()
            .to_string();
        // content follows the author field plus one separator space; a
        // short line (empty content) simply yields an empty string
        let content = String::from_utf8_lossy(line.get(end + 12..).unwrap_or(&[])).into_owned();
        let revision = if rev_str == b"-" {
            None
        } else {
            std::str::from_utf8(rev_str)
                .ok()
                .and_then(|s| s.parse().ok())
        };
        lines.push(BlameLine {
            revision,
            author,
            content,
        });
    }
    lines
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
        // Property change section: in a mixed diff the text hunks are
        // followed by a blank line, "Property changes on: f", a line of
        // underscores, and "## -0,0 +1 ##" property hunk headers. None of
        // these carry text line numbers — reset the counters so the
        // property lines are not numbered with the text hunk's offsets.
        if line.starts_with("Property changes on:") {
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
        if line.starts_with("## ") {
            old_line = None;
            new_line = None;
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
    fn test_parse_version() {
        assert_eq!(parse_version("1.14.5"), Some((1, 14, 5)));
        assert_eq!(parse_version("1.14.5 (r1876290)\n"), Some((1, 14, 5)));
        assert_eq!(parse_version("1.8"), Some((1, 8, 0)));
        assert_eq!(parse_version("???"), None);
        assert_eq!(parse_version(""), None);
        assert!(version_at_least((1, 14, 5), crate::svn::MIN_SVN_VERSION));
        assert!(version_at_least(
            crate::svn::MIN_SVN_VERSION,
            crate::svn::MIN_SVN_VERSION
        ));
        assert!(!version_at_least((1, 7, 99), crate::svn::MIN_SVN_VERSION));
        assert!(version_at_least((2, 0, 0), crate::svn::MIN_SVN_VERSION));
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
Last Changed Author: alice
Last Changed Rev: 1230
Last Changed Date: 2026-01-01 10:00:00 +0000 (Thu, 01 Jan 2026)
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
        assert_eq!(info.repo_root, "https://svn.example.com/repos/proj");
        assert_eq!(info.uuid, "12345678-1234-1234-1234-123456789012");
        assert_eq!(info.last_author, "alice");
        assert_eq!(info.last_rev, 1230);
        assert!(
            info.last_date.starts_with("2026-01-01"),
            "{}",
            info.last_date
        );
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
    fn test_parse_status_path_beginning_with_gt() {
        // A path may begin with '>' (byte 8); only svn's own continuation
        // prefix (exactly six spaces + '>') marks a continuation line
        let out = "?       >new.txt\n      >   tree-conflict description\n";
        let entries = parse_status(out, no_root());
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].status, '?');
        assert_eq!(entries[0].path, ">new.txt");
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
    fn test_parse_log_message_with_longer_dash_line() {
        // A message line with MORE than 72 dashes is content too — only an
        // exact 72-dash line is the entry separator (svn hardcodes 72)
        let dashes80 = "-".repeat(80);
        let out = format!(
            "{sep}\nr7 | alice | 2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026) | 3 lines\n\nbefore\n{dashes80}\nafter\n{sep}\n",
            sep = "-".repeat(72),
        );
        let entries = parse_log(&out);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].message, format!("before\n{dashes80}\nafter"));
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
        let out = b"     3    kenshin fn main() {\n     -          -     indented\n";
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
    fn test_parse_blame_space_author_and_real_cjk_format() {
        // authors with spaces keep the content intact (the 10-byte author
        // column makes this unambiguous)
        let lines = parse_blame(b"     5 Gabi Melma hello\n");
        assert_eq!(lines[0].author, "Gabi Melma");
        assert_eq!(lines[0].content, "hello");

        // real svn output: the author field is exactly 10 bytes — short
        // CJK names are left-padded, long names are byte-truncated
        // (possibly mid-char); a 7-digit revision shifts everything right
        let mut out = Vec::new();
        out.extend_from_slice(b"     3     "); // rev + sep + 4 pad bytes
        out.extend_from_slice("张三".as_bytes()); // 6 bytes: field full
        out.extend_from_slice(b" fn main() {\n");
        // author 张三李四 (12 bytes) truncated to its first 10: the cut
        // lands inside 四 (e5 9b 9b)
        out.extend_from_slice(b"     4 ");
        out.extend_from_slice(&[0xe5, 0xbc, 0xa0, 0xe4, 0xb8, 0x89, 0xe6, 0x9d, 0x8e, 0xe5]);
        out.extend_from_slice(b" x = 1\n");
        out.extend_from_slice(b"1234567 verylongau y = 2\n");
        let lines = parse_blame(&out);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].author, "张三");
        assert_eq!(lines[0].content, "fn main() {");
        // a mid-char cut garbles only the author, never the content
        assert_eq!(lines[1].author, "张三李\u{FFFD}");
        assert_eq!(lines[1].content, "x = 1");
        assert_eq!(lines[2].revision, Some(1234567));
        assert_eq!(lines[2].author, "verylongau");
        assert_eq!(lines[2].content, "y = 2");
    }

    #[test]
    fn test_parse_blame_xml_and_merge() {
        let xml = "\
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<blame>
<target
   path=\"f.txt\">
<entry
   line-number=\"1\">
<commit
   revision=\"1\">
<author>Gabi Melman</author>
<date>2026-08-29T15:43:00.081617Z</date>
</commit>
</entry>
<entry
   line-number=\"2\">
</entry>
<entry
   line-number=\"3\">
<commit
   revision=\"3\">
<author>张 &amp; 三</author>
<date>2026-08-29T15:43:02.062569Z</date>
</commit>
</entry>
</target>
</blame>";
        let authors = parse_blame_xml(xml);
        assert_eq!(authors.len(), 3);
        assert_eq!(authors[0], (1, Some("Gabi Melman".to_string())));
        assert_eq!(authors[1], (2, None)); // uncommitted line: no <commit>
        assert_eq!(authors[2], (3, Some("张 & 三".to_string()))); // entities decoded

        let mut lines = vec![
            BlameLine {
                revision: Some(1),
                author: "Gabi Melma".into(),
                content: "a".into(),
            },
            BlameLine {
                revision: None,
                author: "-".into(),
                content: "b".into(),
            },
            BlameLine {
                revision: Some(3),
                author: "garbled".into(),
                content: "c".into(),
            },
        ];
        merge_blame_authors(&mut lines, &authors);
        assert_eq!(lines[0].author, "Gabi Melman");
        assert_eq!(lines[1].author, "-", "uncommitted keeps the text side");
        assert_eq!(lines[2].author, "张 & 三");
        // absent xml leaves everything untouched
        let mut same = lines.clone();
        merge_blame_authors(&mut same, &[]);
        assert_eq!(same[0].author, "Gabi Melman");
    }

    #[test]
    fn test_merge_blame_authors_matches_by_line_number() {
        // xml entries out of order still land on the right text line
        let mut lines = vec![
            BlameLine {
                revision: Some(1),
                author: "truncated1".into(),
                content: "a".into(),
            },
            BlameLine {
                revision: Some(2),
                author: "truncated2".into(),
                content: "b".into(),
            },
        ];
        let authors = vec![
            (2, Some("Second Author".to_string())),
            (1, Some("First Author".to_string())),
        ];
        merge_blame_authors(&mut lines, &authors);
        assert_eq!(lines[0].author, "First Author");
        assert_eq!(lines[1].author, "Second Author");
    }

    #[test]
    fn test_merge_blame_authors_misaligned_gives_up() {
        let make = || {
            vec![
                BlameLine {
                    revision: Some(1),
                    author: "text-a".into(),
                    content: "a".into(),
                },
                BlameLine {
                    revision: Some(2),
                    author: "text-b".into(),
                    content: "b".into(),
                },
            ]
        };
        // fewer xml entries than text lines: no partial merge
        let mut lines = make();
        merge_blame_authors(&mut lines, &[(1, Some("xml-a".to_string()))]);
        assert_eq!(lines[0].author, "text-a");
        // line number out of range: the two runs disagree, keep text side
        let mut lines = make();
        merge_blame_authors(
            &mut lines,
            &[
                (1, Some("xml-a".to_string())),
                (5, Some("xml-b".to_string())),
            ],
        );
        assert_eq!(lines[0].author, "text-a");
        assert_eq!(lines[1].author, "text-b");
        // a missing line-number attribute parses as 0: same degradation
        let authors = parse_blame_xml("<blame><entry>\n</entry>\n</blame>");
        assert_eq!(authors, vec![(0, None)]);
        let mut lines = make();
        merge_blame_authors(&mut lines, &authors);
        assert_eq!(lines[0].author, "text-a");
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
    #[test]
    fn test_parse_status_props_column() {
        // a property-only change sets column 2 and leaves column 1 blank
        let entries = parse_status(" M      props.txt\n", no_root());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, ' ');
        assert_eq!(entries[0].props_status, 'M');
        assert_eq!(entries[0].path, "props.txt");
    }

    #[test]
    fn test_parse_status_blank_status_columns_skipped() {
        // a line whose status columns are all spaces carries no information
        // (svn prints such padding in some edge outputs); it is skipped
        let entries = parse_status("        padded.txt\nM       real.txt\n", no_root());
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].path, "real.txt");
    }

    #[test]
    fn test_parse_log_entry_without_message() {
        // an empty commit message ("0 lines") yields an empty message, not
        // a swallowed entry
        let out = "\
------------------------------------------------------------------------
r3 | alice | 2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026) | 0 lines
Changed paths:
   M /trunk/a.txt
------------------------------------------------------------------------
";
        let entries = parse_log(out);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].revision, 3);
        assert_eq!(entries[0].message, "");
        assert_eq!(entries[0].changed, vec![('M', "trunk/a.txt".to_string())]);
    }

    #[test]
    fn test_parse_diff_no_newline_note_keeps_line_numbers() {
        // the "\ No newline" note belongs to the previous line and must not
        // consume an old/new line number itself
        let out = "\
Index: f
===================================================================
--- f\t(revision 1)
+++ f\t(working copy)
@@ -1 +1 @@
-old
\\ No newline at end of file
+new
\\ No newline at end of file
";
        let d = parse_diff(out);
        assert_eq!(d.lines.len(), 9, "{:?}", d.lines);
        assert_eq!(d.lines[5].kind, DiffLineKind::Removed);
        assert_eq!(d.lines[5].old, Some(1));
        assert_eq!(d.lines[6].kind, DiffLineKind::Note);
        assert_eq!(d.lines[6].old, None);
        assert_eq!(d.lines[6].new, None);
        // if the note had advanced the counters this would be Some(2)
        assert_eq!(d.lines[7].kind, DiffLineKind::Added);
        assert_eq!(d.lines[7].new, Some(1));
        assert_eq!(d.lines[8].kind, DiffLineKind::Note);
    }

    #[test]
    fn test_parse_hunk_header_with_function_context() {
        // svn/git append the enclosing function after the second @@; the
        // numbers must still parse
        assert_eq!(parse_hunk(" -1 +1 @@ fn foo"), (Some(1), Some(1)));
        assert_eq!(
            parse_hunk(" -10,4 +10,5 @@ impl Bar<T> for"),
            (Some(10), Some(10))
        );
        // ... through a full diff, so following lines are numbered right
        let d = parse_diff("@@ -5 +5 @@ fn foo\n-a\n+b\n");
        assert_eq!(d.lines[1].kind, DiffLineKind::Removed);
        assert_eq!(d.lines[1].old, Some(5));
        assert_eq!(d.lines[2].kind, DiffLineKind::Added);
        assert_eq!(d.lines[2].new, Some(5));
    }

    #[test]
    fn test_parse_blame_empty_output() {
        assert!(parse_blame(b"").is_empty());
        // whitespace-only lines carry no revision token either
        assert!(parse_blame(b"\n   \n").is_empty());
    }

    #[test]
    fn test_parse_status_skips_conflict_summary() {
        // Real svn 1.14 output with conflicts: a "Summary of conflicts:"
        // trailer with indented detail lines. Neither may become an entry
        // ("Summary"[..7] would otherwise parse as status 'S' with path
        // "of conflicts:")
        let out = "\
C       f.txt
?       f.txt.mine
Summary of conflicts:
  Text conflicts: 1
";
        let entries = parse_status(out, no_root());
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].status, 'C');
        assert_eq!(entries[0].path, "f.txt");
        assert_eq!(entries[1].path, "f.txt.mine");
        // property-conflict variant of the trailer
        let out = " C      g.txt\nSummary of conflicts:\n  Property conflicts: 1\n";
        let entries = parse_status(out, no_root());
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].props_status, 'C');
        assert!(entries[0].is_conflicted());
    }

    #[test]
    fn test_parse_log_message_with_exact_separator_and_fake_header() {
        // Real svn 1.14 output for a message containing an exact 72-dash
        // line and a header-looking line: the header's line count bounds
        // the message, so neither truncates the entry nor spawns a
        // phantom revision
        let sep = "-".repeat(72);
        let out = format!(
            "{sep}\nr5 | kenshin | 2026-08-30 11:57:12 +0800 (Sun, 30 Aug 2026) | 4 lines\nChanged paths:\n   M /f.txt\n\nbefore\n{sep}\nr99 | fake | not-a-date | 1 line\nafter\n{sep}\n"
        );
        let entries = parse_log(&out);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].revision, 5);
        assert_eq!(entries[0].line_count, 4);
        assert_eq!(
            entries[0].message,
            format!("before\n{sep}\nr99 | fake | not-a-date | 1 line\nafter")
        );
        assert_eq!(entries[0].changed, vec![('M', "f.txt".to_string())]);
    }

    #[test]
    fn test_parse_log_header_author_with_pipe() {
        // svn does not sanitize the author; one containing " | " must not
        // shift the date/line_count fields
        let out = "\
------------------------------------------------------------------------
r7 | Gabi | Melman | 2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026) | 1 line
msg
------------------------------------------------------------------------
";
        let entries = parse_log(out);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].revision, 7);
        assert_eq!(entries[0].author, "Gabi | Melman");
        assert_eq!(
            entries[0].date,
            "2026-08-26 21:41:52 +0800 (Wed, 26 Aug 2026)"
        );
        assert_eq!(entries[0].line_count, 1);
        assert_eq!(entries[0].message, "msg");
    }

    #[test]
    fn test_parse_diff_property_section() {
        // Real svn 1.14 mixed text+property diff: the property section
        // must not inherit the text hunk's line numbers
        let out = "\
Index: f.txt
===================================================================
--- f.txt\t(revision 1)
+++ f.txt\t(working copy)
@@ -1 +1 @@
-base
+changed

Property changes on: f.txt
___________________________________________________________________
Added: Id
## -0,0 +1 ##
+x
\\ No newline at end of property
";
        let d = parse_diff(out);
        let kinds: Vec<_> = d.lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DiffLineKind::Header,     // Index:
                DiffLineKind::Header,     // ===
                DiffLineKind::FileHeader, // ---
                DiffLineKind::FileHeader, // +++
                DiffLineKind::Hunk,       // @@
                DiffLineKind::Removed,    // -base
                DiffLineKind::Added,      // +changed
                DiffLineKind::Context,    // blank separator line
                DiffLineKind::Header,     // Property changes on:
                DiffLineKind::Context,    // ____ separator
                DiffLineKind::Context,    // Added: Id
                DiffLineKind::Hunk,       // ## -0,0 +1 ##
                DiffLineKind::Added,      // +x
                DiffLineKind::Note,       // \ No newline at end of property
            ],
            "{:?}",
            d.lines
        );
        // text hunk keeps its numbers...
        assert_eq!(d.lines[5].old, Some(1));
        assert_eq!(d.lines[6].new, Some(1));
        // ...but the property section is unnumbered
        for l in &d.lines[8..] {
            assert_eq!(l.old, None, "{l:?}");
            assert_eq!(l.new, None, "{l:?}");
        }
        assert_eq!(d.lines[12].content, "x");
        assert_eq!(d.lines[13].content, "\\ No newline at end of property");
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
