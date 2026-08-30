//! Scrollable diff rendering, shared between the status tab diff pane and
//! the fullscreen diff popup.

use super::EventState;
use super::text_search::highlight_spans;
use super::text_view::{SearchOutcome, TextView};
use crate::svn::models::{DiffLine, DiffLineKind, LogEntry, ParsedDiff};
use crate::svn::parser::{parse_diff, parse_new_file_content};
use crate::ui::{self, style::Theme};
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};

/// Max lines the fixed commit-info header above a diff may occupy, so
/// huge (merge) commit messages cannot eat the whole screen.
pub const DIFF_HEADER_MAX: usize = 5;

/// Header lines for a single-revision diff: `r<N> | author | date` plus
/// message lines, capped at `DIFF_HEADER_MAX`. When the message is
/// longer, the last shown line ends with `…`.
pub fn revision_header(e: &LogEntry) -> Vec<String> {
    let mut out = vec![format!("r{} | {} | {}", e.revision, e.author, e.date)];
    push_capped(&mut out, &e.message);
    out
}

/// Header lines for a combined range diff: `r<from>..r<to> (n commits)`
/// plus the message of the newest revision in the range (when it fits).
/// `entries` are the loaded log entries (newest first); when none of
/// them fall into the range the count is omitted.
pub fn range_header(from: u64, to: u64, entries: &[LogEntry]) -> Vec<String> {
    let in_range: Vec<&LogEntry> = entries
        .iter()
        .filter(|e| (from..=to).contains(&e.revision))
        .collect();
    let mut out = if in_range.is_empty() {
        vec![format!("r{from}..r{to}")]
    } else {
        vec![format!("r{from}..r{to} ({} commits)", in_range.len())]
    };
    if let Some(newest) = in_range.first() {
        push_capped(&mut out, &newest.message);
    }
    out
}

/// Append message lines to `out` without exceeding `DIFF_HEADER_MAX`
/// total lines; the last line gets a `…` marker when truncated.
fn push_capped(out: &mut Vec<String>, message: &str) {
    let room = DIFF_HEADER_MAX.saturating_sub(out.len());
    let lines: Vec<&str> = message.lines().collect();
    let shown = lines.len().min(room);
    for line in &lines[..shown] {
        out.push((*line).to_string());
    }
    if lines.len() > shown
        && let Some(last) = out.last_mut()
    {
        last.push_str(" …");
    }
}

/// A scrollable view of a parsed diff.
pub struct DiffView {
    pub title: String,
    pub parsed: ParsedDiff,
    /// Scroll offsets + incremental search (shared text-view plumbing)
    pub tv: TextView,
    pub pending: bool,
    /// Set when there is nothing to show
    pub empty_reason: Option<String>,
    pub focused: bool,
    /// Fixed commit-info lines above the diff (revision/range diffs);
    /// never scrolled, capped at `DIFF_HEADER_MAX`
    header: Vec<String>,
    /// Search is only consulted when `search_enabled` is set (fullscreen
    /// popup) — the status-tab diff pane leaves search off.
    pub search_enabled: bool,
    /// Column width of one line-number field (≥3), sized to the largest
    /// line number in the diff. Rendering and the h-scroll clamp both
    /// derive the gutter width from this, so they can never disagree.
    num_w: usize,
}

/// Column width of one line-number field: at least 3, widened to fit the
/// largest line number in the diff so 4+-digit numbers stay aligned.
fn line_number_width(parsed: &ParsedDiff) -> usize {
    let max_n = parsed
        .lines
        .iter()
        .filter_map(|dl| dl.old.max(dl.new))
        .max()
        .unwrap_or(0);
    let digits = max_n.checked_ilog10().map_or(1, |d| d as usize + 1);
    digits.max(3)
}

/// Decide whether raw content is a unified diff (vs. plain text, e.g. an
/// unversioned file shown as all-added lines). Only the *start* of the
/// content counts: every file section of real `svn diff` output begins
/// with "Index: ", property-only diffs with "Property changes on:", and
/// patch files (previewed through the diff popup) may be git-format
/// ("diff --git ") or plain `diff -u` output (a "--- "/"+++ " header
/// pair). A hunk header ("@@ ...") mid-file is just text.
fn looks_like_diff(content: &str) -> bool {
    let mut lines = content.trim_start().lines();
    let Some(first) = lines.next() else {
        return false;
    };
    first.starts_with("Index: ")
        || first.starts_with("Property changes on:")
        || first.starts_with("diff --git ")
        || (first.starts_with("--- ") && lines.next().is_some_and(|l| l.starts_with("+++ ")))
}

impl DiffView {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            parsed: ParsedDiff::default(),
            tv: TextView::new(),
            pending: true,
            empty_reason: None,
            focused: true,
            header: Vec::new(),
            search_enabled: false,
            num_w: 3,
        }
    }

    /// Set the fixed commit-info header (capped at `DIFF_HEADER_MAX`).
    pub fn set_header(&mut self, header: Vec<String>) {
        self.header = header.into_iter().take(DIFF_HEADER_MAX).collect();
    }

    /// The fixed commit-info header lines, if any.
    pub fn header(&self) -> &[String] {
        &self.header
    }

    pub fn set_loading(&mut self, title: String) {
        self.title = title;
        self.pending = true;
        self.empty_reason = None;
        self.tv.reset();
    }

    /// Show a placeholder message instead of a diff (e.g. "select a file").
    pub fn set_hint(&mut self, title: String, reason: String) {
        self.title = title;
        self.pending = false;
        self.empty_reason = Some(reason);
        self.parsed = ParsedDiff::default();
        self.num_w = 3;
        self.tv.reset();
    }

    /// Set raw diff text (or raw file content for unversioned files).
    pub fn set_content(&mut self, title: String, content: &str) {
        self.title = title;
        self.pending = false;
        self.tv.reset();
        if content.trim().is_empty() {
            self.empty_reason = Some("No textual diff".to_string());
            self.parsed = ParsedDiff::default();
        } else if looks_like_diff(content) {
            self.parsed = parse_diff(content);
            self.empty_reason = None;
        } else {
            self.parsed = parse_new_file_content(content);
            self.empty_reason = None;
        }
        self.num_w = line_number_width(&self.parsed);
        self.tv.max_width.set(self.compute_max_width());
    }

    /// Display width of the widest rendered line, including the
    /// line-number gutter (" 12  34 │ ", 2*num_w + 3 columns) for
    /// numbered lines.
    fn compute_max_width(&self) -> usize {
        use unicode_width::UnicodeWidthStr;
        let gutter = 2 * self.num_w + 3;
        self.parsed
            .lines
            .iter()
            .map(|dl| {
                let w = UnicodeWidthStr::width(dl.content.as_str());
                match dl.kind {
                    DiffLineKind::Context | DiffLineKind::Added | DiffLineKind::Removed => {
                        w + gutter
                    }
                    _ => w,
                }
            })
            .max()
            .unwrap_or(0)
    }

    pub fn event(&mut self, ev: &Event) -> EventState {
        self.tv.scroll_event(ev, self.parsed.lines.len())
    }

    /// Handle search-related input (`/`, live typing, `n`/`N`).
    ///
    /// Only active when `search_enabled` (fullscreen popup); returns
    /// `Some(consumed)` when the event was search business, `None` so the
    /// caller can fall back to scrolling/closing. Esc is *not* handled
    /// here — it interacts with closing the popup, which the view cannot
    /// do (see `DiffPopup::event`).
    pub fn search_event(&mut self, ev: &Event) -> Option<EventState> {
        if !self.search_enabled {
            return None;
        }
        let lines: Vec<&str> = self
            .parsed
            .lines
            .iter()
            .map(|l| l.content.as_str())
            .collect();
        match self.tv.search_event(ev, &lines, self.tv.scroll.get()) {
            SearchOutcome::Ignored => None,
            SearchOutcome::Consumed => Some(EventState::consumed()),
            SearchOutcome::Reveal(line) => {
                self.tv.scroll.set(line);
                Some(EventState::consumed())
            }
        }
    }
}

/// Build a single styled diff line with line numbers.
///
/// `matches`/`current` highlight incremental-search hits inside the
/// content: `matches` are the byte ranges on this line (from
/// `TextSearch::line_ranges`) and `current` the index of the current
/// match within them (distinct style). `num_w` is the column width of
/// one line-number field (see `line_number_width`).
pub fn diff_line(
    dl: &DiffLine,
    theme: &Theme,
    matches: &[(usize, usize)],
    current: Option<usize>,
    num_w: usize,
) -> Line<'static> {
    let (kind_style, need_numbers) = match dl.kind {
        DiffLineKind::Header => (theme.diff_header, false),
        DiffLineKind::FileHeader => (theme.diff_file_header, false),
        DiffLineKind::Hunk => (theme.diff_hunk, false),
        DiffLineKind::Note => (theme.diff_note, false),
        DiffLineKind::Context => (theme.text, true),
        DiffLineKind::Added => (theme.diff_added, true),
        DiffLineKind::Removed => (theme.diff_removed, true),
    };

    let content = highlight_spans(
        &dl.content,
        kind_style,
        matches,
        current,
        theme.search_hit,
        theme.search_hit_current,
    );
    if !need_numbers {
        return Line::from(content);
    }

    let num = |n: Option<u64>| -> String {
        match n {
            Some(n) => format!("{n:>num_w$}"),
            None => " ".repeat(num_w),
        }
    };
    let mut spans = vec![
        Span::styled(num(dl.old), theme.diff_line_number),
        Span::raw(" "),
        Span::styled(num(dl.new), theme.diff_line_number),
        Span::styled("│ ", theme.diff_line_number),
    ];
    spans.extend(content);
    Line::from(spans)
}

/// Draw a diff into an area with a block title (shared by pane & popup).
pub fn draw_diff_block(f: &mut Frame, area: Rect, view: &DiffView, theme: &Theme) {
    let border = if view.focused {
        theme.border_focused
    } else {
        theme.border_unfocused
    };
    let title = if view.pending {
        format!("{}  {}", view.title, crate::strings::MSG.loading_suffix)
    } else {
        view.title.clone()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if let Some(reason) = &view.empty_reason {
        f.render_widget(
            ratatui::widgets::Paragraph::new(Line::from(Span::styled(reason.clone(), theme.dim))),
            inner,
        );
        return;
    }
    if view.pending {
        f.render_widget(
            ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                crate::strings::MSG.loading,
                theme.dim,
            ))),
            inner,
        );
        return;
    }

    // Fixed commit-info header (revision/range diffs): drawn above the
    // scrollable area, dimmed and separated by a rule, never scrolled.
    let inner = if view.header.is_empty() {
        inner
    } else {
        // +1 for the separator rule
        let header_h = (view.header.len() as u16 + 1).min(inner.height);
        let header_area = Rect::new(inner.x, inner.y, inner.width, header_h);
        let mut header_lines: Vec<Line> = view
            .header
            .iter()
            .map(|l| Line::from(Span::styled(l.clone(), theme.dim)))
            .collect();
        header_lines.push(Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            theme.dim,
        )));
        ui::render_lines(f, header_area, &header_lines, 0, &[]);
        Rect::new(
            inner.x,
            inner.y + header_h,
            inner.width,
            inner.height - header_h,
        )
    };

    // Virtualized rendering: only the visible window of lines is built, so
    // drawing a huge diff costs O(screen height), not O(diff size).
    // While a search is active the bottom row is a `/pattern [x/y]` footer.
    let total = view.parsed.lines.len();
    let (inner, footer, scroll) = view.tv.layout(inner, total);
    let end = (scroll + inner.height as usize).min(total);
    let mut lines = Vec::with_capacity(end - scroll);
    for i in scroll..end {
        let (ranges, current) = view.tv.search.line_ranges(i);
        lines.push(diff_line(
            &view.parsed.lines[i],
            theme,
            &ranges,
            current,
            view.num_w,
        ));
    }
    ui::render_lines_h(f, inner, &lines, 0, &[], view.tv.hscroll.get());
    if let Some(footer) = footer {
        view.tv.draw_search_footer(f, footer, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;
    use crate::ui::style::Theme;
    use crossterm::event::KeyCode;

    const DIFF: &str = "\
Index: Cargo.toml
===================================================================
--- Cargo.toml\t(revision 1)
+++ Cargo.toml\t(working copy)
@@ -1 +1,2 @@
 version = 1
+extra
\\ No newline at end of file
";

    #[test]
    fn header_builders_cap_at_five_lines() {
        let e = LogEntry {
            revision: 7,
            author: "alice".into(),
            date: "2026-01-01".into(),
            line_count: 9,
            changed: vec![],
            message: "line1\nline2\nline3\nline4\nline5\nline6\nline7".into(),
        };
        let h = revision_header(&e);
        assert_eq!(h.len(), DIFF_HEADER_MAX);
        assert_eq!(h[0], "r7 | alice | 2026-01-01");
        assert_eq!(h[1], "line1");
        // truncated: the last shown message line gets the ellipsis
        assert_eq!(h[4], "line4 …");

        // a short message is not truncated
        let short = LogEntry {
            message: "only line".into(),
            line_count: 1,
            ..e
        };
        let h2 = revision_header(&short);
        assert_eq!(h2, vec!["r7 | alice | 2026-01-01", "only line"]);
    }

    #[test]
    fn range_header_counts_commits_and_summarizes_newest() {
        let mk = |rev: u64, msg: &str| LogEntry {
            revision: rev,
            author: "a".into(),
            date: "d".into(),
            line_count: 1,
            changed: vec![],
            message: msg.to_string(),
        };
        // log order is newest first
        let entries = vec![mk(9, "newest\nsecond line"), mk(5, "mid"), mk(3, "old")];
        let h = range_header(3, 9, &entries);
        assert_eq!(h[0], "r3..r9 (3 commits)");
        assert_eq!(h[1], "newest");
        assert_eq!(h[2], "second line");
        assert!(h.len() <= DIFF_HEADER_MAX);

        // long newest message is capped
        let long = vec![mk(9, "1\n2\n3\n4\n5\n6"), mk(3, "x")];
        let h2 = range_header(3, 9, &long);
        assert_eq!(h2.len(), DIFF_HEADER_MAX);
        assert!(h2.last().unwrap().ends_with(" …"));

        // no loaded entries in range → no count, just the range
        let h3 = range_header(50, 60, &entries);
        assert_eq!(h3, vec!["r50..r60"]);
    }

    #[test]
    fn header_is_fixed_and_does_not_scroll_away() {
        let mut v = DiffView::new("t");
        v.set_content("t".into(), DIFF);
        v.set_header(vec![
            "r7 | alice | 2026-01-01".to_string(),
            "commit message".to_string(),
        ]);
        // scroll to the bottom: the header must still be drawn
        v.tv.scroll.set(8);
        let t = ts::render(60, 12, |f| {
            draw_diff_block(f, Rect::new(0, 0, 60, 12), &v, &Theme::default());
        });
        let s = ts::dump(&t);
        assert!(s.contains("r7 | alice | 2026-01-01"), "{s}");
        assert!(s.contains("commit message"), "{s}");
        // scrolled: the first diff lines are gone, later ones visible
        assert!(!s.contains("Index: Cargo.toml"), "{s}");
        assert!(s.contains("extra"), "{s}");
    }

    #[test]
    fn set_header_caps_at_max() {
        let mut v = DiffView::new("t");
        v.set_header((0..10).map(|i| format!("line {i}")).collect());
        assert_eq!(v.header().len(), DIFF_HEADER_MAX);
        let mut v2 = DiffView::new("t");
        v2.set_header(Vec::new());
        assert!(v2.header().is_empty());
    }

    #[test]
    fn horizontal_scroll_shifts_long_lines() {
        let mut v = DiffView::new("t");
        let content = format!("Index: f\n@@ -1 +1 @@\n+{}\n", "x".repeat(200));
        v.set_content("t".into(), &content);
        assert_eq!(v.tv.hscroll.get(), 0);
        // l/h move the view by 8 columns
        v.event(&ts::key(KeyCode::Char('l')));
        assert_eq!(v.tv.hscroll.get(), 8);
        v.event(&ts::key(KeyCode::Char('l')));
        v.event(&ts::key(KeyCode::Char('h')));
        assert_eq!(v.tv.hscroll.get(), 8);
        // rendered: skipping 8 columns of the 9-wide gutter leaves one
        // space before the '+' (row 3 = the added line)
        let t = ts::render(30, 6, |f| {
            draw_diff_block(f, Rect::new(0, 0, 30, 6), &v, &Theme::default());
        });
        let buf = t.backend().buffer();
        // skipping 8 columns of the 9-wide gutter leaves one space before
        // the content ('+' is stripped into the line kind by the parser)
        assert_eq!(buf[(1, 3)].symbol(), " ");
        assert_eq!(buf[(2, 3)].symbol(), "x");
        // scrolling far right clamps at max_width - inner width
        for _ in 0..40 {
            v.event(&ts::key(KeyCode::Char('l')));
        }
        ts::render(30, 6, |f| {
            draw_diff_block(f, Rect::new(0, 0, 30, 6), &v, &Theme::default());
        });
        assert_eq!(v.tv.hscroll.get(), 209 - 28);
        // new content resets both offsets
        v.set_content("t".into(), &content);
        assert_eq!(v.tv.hscroll.get(), 0);
        assert_eq!(v.tv.scroll.get(), 0);
    }

    #[test]
    fn parses_real_diff_with_numbers() {
        let mut v = DiffView::new("t");
        v.set_content("Cargo.toml".into(), DIFF);
        assert!(!v.pending);
        assert!(v.empty_reason.is_none());
        assert_eq!(v.parsed.lines.len(), 8);
        assert_eq!(v.parsed.lines[0].kind, DiffLineKind::Header);
        assert_eq!(v.parsed.lines[4].kind, DiffLineKind::Hunk);
        let ctx_line = &v.parsed.lines[5];
        assert_eq!(ctx_line.kind, DiffLineKind::Context);
        assert_eq!(ctx_line.new, Some(1));
        let add = &v.parsed.lines[6];
        assert_eq!(add.kind, DiffLineKind::Added);
        assert_eq!(add.new, Some(2));
        let theme = Theme::default();
        let lines: Vec<Line> = v
            .parsed
            .lines
            .iter()
            .map(|dl| diff_line(dl, &theme, &[], None, v.num_w))
            .collect();
        assert!(lines.len() >= 7);
        // line numbers rendered
        let rendered = lines.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        assert!(
            rendered
                .iter()
                .any(|l| l.contains("1") && l.contains("version")),
            "{rendered:?}"
        );
    }

    #[test]
    fn new_file_content_and_empty() {
        let mut v = DiffView::new("t");
        v.set_content("new.txt".into(), "line1\nline2\n");
        assert!(v.empty_reason.is_none());
        assert_eq!(v.parsed.lines.len(), 2);
        assert_eq!(v.parsed.lines[1].new, Some(2));
        assert_eq!(v.parsed.lines[1].kind, DiffLineKind::Added);

        let mut v2 = DiffView::new("t");
        v2.set_content("x".into(), "   \n  ");
        assert!(v2.empty_reason.is_some());
    }

    #[test]
    fn property_changes_treated_as_diff() {
        let mut v = DiffView::new("t");
        v.set_content(
            "p".into(),
            "Property changes on: f\n___\nAdded: svn:executable\n",
        );
        assert!(v.empty_reason.is_none());
        assert_eq!(v.parsed.lines.len(), 3);
    }

    #[test]
    fn at_at_mid_line_is_not_a_diff() {
        // raw unversioned text containing "@@" must render as all-added
        // lines, not be misparsed as a diff
        let mut v = DiffView::new("t");
        v.set_content("notes.txt".into(), "mail me at foo@@bar\nplain line\n");
        assert!(v.empty_reason.is_none());
        assert_eq!(v.parsed.lines.len(), 2);
        for line in &v.parsed.lines {
            assert_eq!(line.kind, DiffLineKind::Added);
        }
        assert_eq!(v.parsed.lines[1].new, Some(2));
        // a real diff still parses
        let mut v2 = DiffView::new("t");
        v2.set_content("Cargo.toml".into(), DIFF);
        assert!(v2.parsed.lines.iter().any(|l| l.kind == DiffLineKind::Hunk));
    }

    #[test]
    fn at_at_line_start_inside_plain_text_is_not_a_diff() {
        // plain text whose *first* line is ordinary but a middle line
        // starts with "@@ " (patch excerpt, mail quote) must render as
        // all-added lines, not be misparsed as a diff
        let mut v = DiffView::new("t");
        v.set_content(
            "notes.txt".into(),
            "patch draft:\n@@ -1 +1 @@\n+not a diff line\n",
        );
        assert!(v.empty_reason.is_none());
        assert_eq!(v.parsed.lines.len(), 3);
        for (i, line) in v.parsed.lines.iter().enumerate() {
            assert_eq!(line.kind, DiffLineKind::Added);
            assert_eq!(line.new, Some(i as u64 + 1));
        }
        // the "@@ " line keeps its characters (no stripped prefix)
        assert_eq!(v.parsed.lines[1].content, "@@ -1 +1 @@");
        assert_eq!(v.parsed.lines[2].content, "+not a diff line");
        // rendered with line numbers, content intact
        let t = ts::render(40, 6, |f| {
            draw_diff_block(f, Rect::new(0, 0, 40, 6), &v, &Theme::default());
        });
        let s = ts::dump(&t);
        assert!(s.contains("@@ -1 +1 @@"), "{s}");
        assert!(s.contains("+not a diff line"), "{s}");
    }

    #[test]
    fn patch_files_without_index_header_are_still_diffs() {
        // patch preview feeds raw patch files into the diff popup: both
        // git-format and plain `diff -u` patches lack the "Index: " header
        let mut v = DiffView::new("t");
        v.set_content(
            "p.patch".into(),
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n",
        );
        assert!(v.parsed.lines.iter().any(|l| l.kind == DiffLineKind::Hunk));
        let mut v2 = DiffView::new("t");
        v2.set_content("q.patch".into(), "--- f.orig\n+++ f\n@@ -1 +1 @@\n-a\n+b\n");
        assert!(v2.parsed.lines.iter().any(|l| l.kind == DiffLineKind::Hunk));
        // but YAML front matter ("---" followed by a key) is plain text
        let mut v3 = DiffView::new("t");
        v3.set_content("doc.md".into(), "---\ntitle: hello\n---\nbody\n");
        assert!(
            v3.parsed
                .lines
                .iter()
                .all(|l| l.kind == DiffLineKind::Added),
            "markdown front matter must not parse as a diff"
        );
    }

    #[test]
    fn four_digit_line_numbers_widen_gutter_and_hscroll_reaches_line_end() {
        let mut v = DiffView::new("t");
        let content = format!("Index: f\n@@ -1000 +1000 @@\n {}\n", "x".repeat(200));
        v.set_content("t".into(), &content);
        // 4-digit line numbers: gutter = 2*4 + 3 = 11 columns, and the
        // h-scroll clamp accounts for the full gutter
        assert_eq!(v.tv.max_width.get(), 200 + 11);
        let t = ts::render(40, 6, |f| {
            draw_diff_block(f, Rect::new(0, 0, 40, 6), &v, &Theme::default());
        });
        let buf = t.backend().buffer();
        // the context line is row 3 (border + "Index: f" + hunk header);
        // the gutter is right-aligned: "1000 1000 │ xxxx"
        assert_eq!(buf[(1, 3)].symbol(), "1");
        assert_eq!(buf[(4, 3)].symbol(), "0");
        assert_eq!(buf[(5, 3)].symbol(), " ");
        assert_eq!(buf[(6, 3)].symbol(), "1");
        assert_eq!(buf[(10, 3)].symbol(), "│");
        assert_eq!(buf[(12, 3)].symbol(), "x");
        // scrolling far right clamps at max_width - inner width (38) and
        // the line tail is actually reachable
        for _ in 0..40 {
            v.event(&ts::key(KeyCode::Char('l')));
        }
        let t = ts::render(40, 6, |f| {
            draw_diff_block(f, Rect::new(0, 0, 40, 6), &v, &Theme::default());
        });
        assert_eq!(v.tv.hscroll.get(), 211 - 38);
        let buf = t.backend().buffer();
        assert_eq!(buf[(38, 3)].symbol(), "x", "line end must be reachable");
        assert_eq!(buf[(1, 3)].symbol(), "x", "no gutter residue when scrolled");
    }

    #[test]
    fn scrolling_clamps() {
        let mut v = DiffView::new("t");
        v.set_content("t".into(), DIFF);
        v.event(&ts::key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(v.tv.scroll.get(), 1);
        v.event(&ts::key(crossterm::event::KeyCode::Char('G')));
        assert_eq!(v.tv.scroll.get(), 8);
        v.event(&ts::key(crossterm::event::KeyCode::Char('g')));
        assert_eq!(v.tv.scroll.get(), 0);
        v.event(&ts::key(crossterm::event::KeyCode::PageDown));
        assert_eq!(v.tv.scroll.get(), 8); // bounded by content
        v.event(&ts::key(crossterm::event::KeyCode::PageUp));
        assert_eq!(v.tv.scroll.get(), 0);
        // unknown key not consumed
        let state = v.event(&ts::key(crossterm::event::KeyCode::Char('x')));
        assert!(!state.consumed);
        // scrolling is bounded by the content length
        v.event(&ts::key(crossterm::event::KeyCode::Char('G')));
        v.event(&ts::key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(v.tv.scroll.get(), 8);
        let _ = ts::render(50, 4, |f| {
            draw_diff_block(f, Rect::new(0, 0, 50, 4), &v, &Theme::default());
        });
    }

    #[test]
    fn search_highlights_stay_aligned_with_hscroll() {
        // search active + horizontal scroll at the same time: the match
        // highlight must follow the sliced text, not the pre-slice columns
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "aaaa needle zzzz and some more text\nplain\n");
        v.search_enabled = true;
        v.search_event(&ts::key(KeyCode::Char('/')));
        for ch in "needle".chars() {
            v.search_event(&ts::key(KeyCode::Char(ch)));
        }
        v.search_event(&ts::key(KeyCode::Enter));
        assert_eq!(v.tv.search.match_count(), 1);
        // skip 8 columns of the 9-wide line-number gutter
        v.event(&ts::key(KeyCode::Char('l')));
        assert_eq!(v.tv.hscroll.get(), 8);
        let theme = Theme::default();
        let t = ts::render(30, 6, |f| {
            draw_diff_block(f, Rect::new(0, 0, 30, 6), &v, &theme);
        });
        let buf = t.backend().buffer();
        // 'needle' sits at content column 5..11; on screen: border(1) +
        // gutter rest(1) + 5 = 7, and the single match is the current one
        let hit_bg = theme.search_hit_current.bg.unwrap();
        for x in 7..13 {
            assert_eq!(buf[(x, 1)].bg, hit_bg, "cell {x} must be highlighted");
        }
        assert_ne!(buf[(6, 1)].bg, hit_bg, "the match must not shift left");
        assert_eq!(buf[(6, 1)].symbol(), " ");
        assert_eq!(buf[(7, 1)].symbol(), "n");
        // the search footer is drawn while scrolled
        let s = ts::dump(&t);
        assert!(s.contains("/needle  [1/1]"), "{s}");
    }

    #[test]
    fn cjk_content_horizontal_scroll_aligns_cells() {
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "修复中文注释 here with more text\nplain\n");
        let theme = Theme::default();
        // no hscroll: content starts right after the 9-column gutter
        let t = ts::render(20, 5, |f| {
            draw_diff_block(f, Rect::new(0, 0, 20, 5), &v, &theme);
        });
        let buf = t.backend().buffer();
        assert_eq!(buf[(10, 1)].symbol(), "修");
        assert_eq!(buf[(12, 1)].symbol(), "复");
        // cut at a column boundary: 修复 (4 columns) are gone, 中 starts it
        v.tv.hscroll.set(13); // 9 gutter + 4 content columns
        let t = ts::render(20, 5, |f| {
            draw_diff_block(f, Rect::new(0, 0, 20, 5), &v, &theme);
        });
        let buf = t.backend().buffer();
        assert_eq!(buf[(1, 1)].symbol(), "中");
        assert_eq!(buf[(3, 1)].symbol(), "文");
        // cut through the middle of 修: the straddling char collapses to a
        // single space instead of rendering a half glyph
        v.tv.hscroll.set(10);
        let t = ts::render(20, 5, |f| {
            draw_diff_block(f, Rect::new(0, 0, 20, 5), &v, &theme);
        });
        let buf = t.backend().buffer();
        assert_eq!(buf[(1, 1)].symbol(), " ");
        assert_eq!(buf[(2, 1)].symbol(), "复");
        assert_eq!(buf[(4, 1)].symbol(), "中");
        let s = ts::dump(&t);
        assert!(!s.contains('修'), "{s}");
    }

    #[test]
    fn draw_pending_empty_and_content() {
        let mut v = DiffView::new("Diff");
        v.pending = true;
        let t1 = ts::render(50, 8, |f| {
            draw_diff_block(f, Rect::new(0, 0, 50, 8), &v, &Theme::default());
        });
        assert!(ts::dump(&t1).contains("Loading"));

        v.set_hint("Diff".into(), "no content".into());
        let t2 = ts::render(50, 8, |f| {
            draw_diff_block(f, Rect::new(0, 0, 50, 8), &v, &Theme::default());
        });
        assert!(ts::dump(&t2).contains("no content"));

        v.set_content("Cargo.toml".into(), DIFF);
        let t3 = ts::render(80, 20, |f| {
            draw_diff_block(f, Rect::new(0, 0, 80, 20), &v, &Theme::default());
        });
        let s = ts::dump(&t3);
        assert!(s.contains("Cargo.toml"), "{s}");
        assert!(s.contains("extra"), "{s}");
        assert!(s.contains("version = 1"), "{s}");
    }
}
