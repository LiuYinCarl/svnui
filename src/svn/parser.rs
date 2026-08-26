//! Parsers for `svn status` / `svn log` / `svn blame` / `svn diff` output.

use super::models::{BlameLine, DiffLine, DiffLineKind, LogEntry, ParsedDiff, StatusEntry};

/// Parse the plain-text output of `svn status`.
///
/// Layout (verified against svn 1.14): seven single-character columns
/// (text, props, locked, copied+add, switched, unused, tree-conflict)
/// followed by the path.
pub fn parse_status(output: &str) -> Vec<StatusEntry> {
    let mut entries = Vec::new();
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        // Skip continuation lines of move operations (" >   path")
        if line.starts_with('>') {
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut cols = [b' '; 7];
        for col in cols.iter_mut() {
            if i < bytes.len() {
                *col = bytes[i];
                i += 1;
            }
        }
        // Skip the space between columns and path
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }
        let path = line[i..].to_string();
        if path.is_empty() {
            continue;
        }
        let text = cols[0] as char;
        let props = cols[1] as char;
        let tree_conflict = cols[6] as char;
        if text == ' ' && props == ' ' && tree_conflict == ' ' {
            continue;
        }
        let is_dir = std::path::Path::new(&path).is_dir();
        entries.push(StatusEntry {
            status: text,
            props_status: props,
            tree_conflict,
            path,
            is_dir,
        });
    }
    entries
}

/// Parse `svn log -v` output.
pub fn parse_log(output: &str) -> Vec<LogEntry> {
    const SEP: &str = "------------------------------------";
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

        if line.starts_with("Changed paths:") {
            in_message = false;
            continue;
        }

        // Changed path line: "   M /trunk/foo.txt" (3 leading spaces, '/')
        if !in_message && line.starts_with("   ") {
            let trimmed = line.trim_start();
            let chars: Vec<char> = trimmed.chars().collect();
            if chars.len() > 2 && chars[1] == ' ' && trimmed[2..].starts_with('/') {
                let action = chars[0];
                if matches!(action, 'M' | 'A' | 'D' | 'R' | 'I' | 'X' | 'C' | '?') {
                    let path = trimmed[2..].trim().trim_start_matches('/').to_string();
                    if let Some(e) = current.as_mut() {
                        e.changed.push((action, path));
                    }
                    continue;
                }
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
pub fn parse_blame(output: &str) -> Vec<BlameLine> {
    let mut lines = Vec::new();
    for raw in output.lines() {
        let line = raw.trim_end_matches('\r');
        if line.len() < 18 {
            continue;
        }
        let rev_str = line[0..6].trim();
        let author = line[7..17].trim().to_string();
        // position 17 is the single separator space; keep the file's own
        // leading indentation in the content
        let content = line[18..].to_string();
        let revision = if rev_str == "-" {
            None
        } else {
            rev_str.parse::<u64>().ok()
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
        if line.starts_with("--- ") || line.starts_with("+++ ") {
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

    #[test]
    fn test_parse_status() {
        let out = "M       Cargo.toml\n?       newfile.txt\nA  +    src/foo.rs\n";
        let entries = parse_status(out);
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
        let entries = parse_status("!       gone/dir\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, '!');
        assert!(!entries[0].is_dir);
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
}
