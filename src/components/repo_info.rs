//! Repo info popup content (global `i`): local `svn info`, the remote
//! HEAD comparison, and a working-copy change summary.

use crate::svn::models::SvnInfo;
use crate::ui::style::Theme;
use ratatui::text::{Line, Span};

/// Compose the repo-info popup lines. `changed_files` is the status
/// tree's `(status char, path)` list and `staged_count` its commit-set
/// size — the function takes those two values instead of the component
/// so it stays a pure formatter. Important values are styled: section
/// headers bold, revisions yellow, authors cyan, the behind/up-to-date
/// state yellow/green, status counts in their usual status colors.
pub fn repo_info_lines(
    local: &SvnInfo,
    head: Option<&SvnInfo>,
    changed_files: &[(char, String)],
    staged_count: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let section = |out: &mut Vec<Line<'static>>, title: &str| {
        if !out.is_empty() {
            out.push(Line::default());
        }
        out.push(Line::from(Span::styled(
            title.to_string(),
            theme.diff_header,
        )));
    };
    let field = |label: &str, value: Span<'static>| {
        Line::from(vec![
            Span::styled(format!("  {label:<15}"), theme.dim),
            value,
        ])
    };
    let changed_spans = |prefix: &str, rev: u64, author: &str, date: &str| {
        Line::from(vec![
            Span::styled(format!("  {prefix:<15}"), theme.dim),
            Span::styled(format!("r{rev}"), theme.log_revision),
            Span::styled(" by ", theme.dim),
            Span::styled(author.to_string(), theme.log_author),
            Span::styled(format!(" ({date})"), theme.dim),
        ])
    };

    section(&mut out, "Working copy");
    out.push(field(
        "Path:",
        Span::styled(local.wc_root.clone(), theme.text),
    ));
    out.push(field("URL:", Span::styled(local.url.clone(), theme.text)));
    out.push(field(
        "Branch:",
        Span::styled(local.branch_label().to_string(), theme.log_author),
    ));
    out.push(field(
        "Revision:",
        Span::styled(format!("r{}", local.revision), theme.log_revision),
    ));
    if local.last_rev > 0 {
        out.push(changed_spans(
            "Last changed:",
            local.last_rev,
            &local.last_author,
            &local.last_date,
        ));
    }

    section(&mut out, "Repository");
    out.push(field(
        "Root:",
        Span::styled(local.repo_root.clone(), theme.text),
    ));
    out.push(field("UUID:", Span::styled(local.uuid.clone(), theme.text)));
    match head {
        Some(h) => {
            // SVN wcs are mixed-revision; this compares the root BASE
            let state = if h.revision > local.revision {
                Span::styled(
                    format!(
                        "  (working copy is {} revisions behind)",
                        h.revision - local.revision
                    ),
                    theme.status_modified,
                )
            } else {
                Span::styled("  (up to date)".to_string(), theme.status_added)
            };
            out.push(Line::from(vec![
                Span::styled(format!("  {:<15}", "HEAD:"), theme.dim),
                Span::styled(format!("r{}", h.revision), theme.log_revision),
                state,
            ]));
            if h.last_rev > 0 {
                out.push(changed_spans(
                    "Last commit:",
                    h.last_rev,
                    &h.last_author,
                    &h.last_date,
                ));
            }
        }
        None => {
            out.push(field(
                "HEAD:",
                Span::styled("unknown (repository unreachable)".to_string(), theme.error),
            ));
        }
    }

    section(&mut out, "Working copy changes");
    let mut counts: std::collections::BTreeMap<char, usize> = Default::default();
    for (c, _) in changed_files {
        *counts.entry(*c).or_default() += 1;
    }
    let labels = [
        ('M', "modified"),
        ('A', "added"),
        ('D', "deleted"),
        ('C', "conflicted"),
        ('!', "missing"),
        ('?', "unversioned"),
    ];
    let mut known = 0;
    // (text, style of the count) pairs joined by dim separators
    let mut parts: Vec<(String, Option<char>)> = Vec::new();
    for (c, label) in labels {
        if let Some(n) = counts.get(&c) {
            known += n;
            parts.push((format!("{n} {label}"), Some(c)));
        }
    }
    let total: usize = counts.values().sum();
    if total > known {
        parts.push((format!("{} other", total - known), None));
    }
    if parts.is_empty() {
        out.push(Line::from(Span::styled(
            "  clean".to_string(),
            theme.status_added,
        )));
    } else {
        let mut spans = vec![Span::raw("  ")];
        for (i, (text, c)) in parts.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(", ", theme.dim));
            }
            let style = c.map(|c| theme.status_style(c)).unwrap_or(theme.text);
            spans.push(Span::styled(text.clone(), style));
        }
        out.push(Line::from(spans));
    }
    if staged_count > 0 {
        out.push(Line::from(vec![
            Span::styled("  Staged for commit: ".to_string(), theme.dim),
            Span::styled(staged_count.to_string(), theme.diff_added),
        ]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Context;
    use crate::components::status_tree::StatusTreeComponent;
    use crate::queue::Queue;
    use crate::svn::models::StatusEntry;

    fn test_info() -> SvnInfo {
        SvnInfo {
            url: "file:///repo/trunk".into(),
            branch: "trunk".into(),
            revision: 3,
            wc_root: "/home/user/wc".into(),
            repo_root: "file:///repo".into(),
            uuid: "12345678-1234-1234-1234-123456789012".into(),
            last_author: "alice".into(),
            last_rev: 3,
            last_date: "2026-01-01 10:00:00 +0000".into(),
        }
    }

    fn entry(status: char, path: &str) -> StatusEntry {
        StatusEntry {
            status,
            props_status: ' ',
            tree_conflict: ' ',
            path: path.to_string(),
            is_dir: std::path::Path::new(path).is_dir(),
        }
    }

    fn bare_tree() -> StatusTreeComponent {
        let ctx = Context {
            queue: Queue::new(),
            theme: Theme::default(),
        };
        StatusTreeComponent::new(&ctx)
    }

    fn lines_text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn repo_info_lines_clean_and_up_to_date() {
        let tree = bare_tree();
        let theme = Theme::default();
        // head not newer than the local revision → "(up to date)"
        let mut head = test_info();
        head.revision = 3;
        let text = lines_text(&repo_info_lines(
            &test_info(),
            Some(&head),
            &tree.changed_files(),
            tree.staged_count(),
            &theme,
        ));
        assert!(text.contains("HEAD:          r3"), "{text}");
        assert!(text.contains("(up to date)"), "{text}");
        assert!(!text.contains("revisions behind"), "{text}");
        // no changed files → the summary line says "clean"
        assert!(text.contains("clean"), "{text}");
        assert!(!text.contains("Staged for commit"), "{text}");
    }

    #[test]
    fn repo_info_lines_unknown_statuses_counted_as_other() {
        let mut tree = bare_tree();
        // status chars without a label ('I' ignored, '~' obstructed)
        tree.update(vec![entry('I', "ignored.o"), entry('~', "blocked.txt")]);
        let text = lines_text(&repo_info_lines(
            &test_info(),
            Some(&test_info()),
            &tree.changed_files(),
            tree.staged_count(),
            &Theme::default(),
        ));
        assert!(text.contains("2 other"), "{text}");
        // no labeled breakdown for unknown statuses
        assert!(!text.contains("modified"), "{text}");
    }

    #[test]
    fn repo_info_lines_omit_last_changed_when_rev_zero() {
        let tree = bare_tree();
        // an uncommitted / fresh node has no last-changed triple
        let mut local = test_info();
        local.last_rev = 0;
        local.last_author = String::new();
        local.last_date = String::new();
        let text = lines_text(&repo_info_lines(
            &local,
            Some(&test_info()),
            &tree.changed_files(),
            tree.staged_count(),
            &Theme::default(),
        ));
        assert!(!text.contains("Last changed:"), "{text}");
        // the head's own last-commit line is independent and still shown
        assert!(text.contains("Last commit:"), "{text}");
    }
}
