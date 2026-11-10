//! Scrollable diff rendering, shared between the status tab diff pane and
//! the fullscreen diff popup.

use super::EventState;
use super::text_search::highlight_spans;
use super::text_view::{SearchOutcome, TextView};
use crate::keys::{KeyAction, key_match};
use crate::svn::models::{DiffLine, DiffLineKind, LogEntry, ParsedDiff, PreparedDiff, WsMark};
use crate::ui::{self, style::Theme};
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
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
    /// The complete diff, parked while [`DiffView::ws_filter`] hides part
    /// of it, so toggling the filter back is a move instead of a re-parse.
    full: Option<ParsedDiff>,
    /// Filtered row -> row in `full`, so toggling keeps the reader's place.
    filter_rows: Vec<usize>,
    /// Show only whitespace-only changes (`w`).
    pub ws_filter: bool,
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
            full: None,
            filter_rows: Vec::new(),
            ws_filter: false,
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

    /// Set raw diff text (or raw file content for unversioned files),
    /// parsed synchronously. Repository diffs use [`DiffView::set_prepared`]
    /// so large diffs are never parsed on the UI thread.
    pub fn set_content(&mut self, title: String, content: &str) {
        self.set_prepared(title, crate::svn::parser::parse_diff_content(content));
    }

    /// Install a diff already parsed on the svn worker thread.
    pub fn set_prepared(&mut self, title: String, prepared: PreparedDiff) {
        self.title = title;
        self.pending = false;
        self.tv.reset();
        self.full = None;
        self.filter_rows.clear();
        self.parsed = prepared.parsed;
        self.empty_reason = prepared.empty_reason;
        prepare_content(&mut self.parsed.lines);
        if self.ws_filter {
            self.apply_ws_filter();
        }
        self.num_w = line_number_width(&self.parsed);
        self.tv.max_width.set(self.compute_max_width());
    }

    /// Show only whitespace-only changes, or the whole diff again (`w`).
    ///
    /// Filtering rebuilds the displayed line list instead of hiding rows at
    /// draw time, so scrolling, search and the `[`/`]` file jumps keep
    /// working on plain line indices; the full diff is parked for the way
    /// back. The reader's place survives the toggle: the top row is
    /// translated through the row map, the horizontal offset is kept (the
    /// draw-time clamp handles a narrower line set) and an active search is
    /// recomputed against the new rows.
    pub fn toggle_ws_filter(&mut self) {
        // the reader's place is a line of the *underlying* diff: remember
        // it, rebuild the displayed rows, and scroll back to it — a toggle
        // in the middle of a long diff must not jump to the top
        let anchor = if self.ws_filter {
            self.filter_rows.get(self.tv.scroll.get()).copied()
        } else {
            Some(self.tv.scroll.get())
        };
        if self.ws_filter {
            if let Some(full) = self.full.take() {
                self.parsed = full;
                self.ws_filter = false;
            }
        } else {
            self.apply_ws_filter();
            self.ws_filter = true;
        }
        let anchor = anchor.unwrap_or(0);
        self.tv.scroll.set(if self.ws_filter {
            // first remaining line at or after the old top row
            self.filter_rows
                .iter()
                .position(|&row| row >= anchor)
                .unwrap_or(self.filter_rows.len().saturating_sub(1))
        } else {
            anchor
        });
        self.num_w = line_number_width(&self.parsed);
        self.tv.max_width.set(self.compute_max_width());
        // keep an active search alive: the pattern still applies, only the
        // lines under it changed
        if !self.tv.search.pattern().is_empty() {
            let lines: Vec<&str> = self
                .parsed
                .lines
                .iter()
                .map(|l| l.content.as_str())
                .collect();
            self.tv.search.recompute(&lines, self.tv.scroll.get());
        }
    }

    /// Park the full diff and display only its whitespace-only changes.
    fn apply_ws_filter(&mut self) {
        let full = std::mem::take(&mut self.parsed);
        (self.parsed, self.filter_rows) = filter_ws_only(&full);
        self.full = Some(full);
    }

    /// Path of the file section the given line belongs to, for the
    /// whole-file view of a multi-file diff: the closest `Index: <path>`
    /// header above it (svn prints one per file) or, for a property-only
    /// section that has no `Index:` line, its `Property changes on: <path>`.
    pub fn file_at(&self, line: usize) -> Option<String> {
        let end = (line + 1).min(self.parsed.lines.len());
        self.parsed.lines[..end].iter().rev().find_map(|dl| {
            if dl.kind != DiffLineKind::Header {
                return None;
            }
            ["Index: ", "Property changes on: "]
                .iter()
                .find_map(|prefix| dl.content.strip_prefix(prefix).map(str::to_string))
        })
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
                if is_content_line(dl.kind) {
                    w + gutter
                } else {
                    w
                }
            })
            .max()
            .unwrap_or(0)
    }

    pub fn event(&mut self, ev: &Event) -> EventState {
        if let Event::Key(k) = ev {
            if key_match(k, KeyAction::NextDiffFile) {
                self.jump_file(true);
                return EventState::consumed();
            }
            if key_match(k, KeyAction::PrevDiffFile) {
                self.jump_file(false);
                return EventState::consumed();
            }
            if key_match(k, KeyAction::ToggleWhitespace) {
                self.toggle_ws_filter();
                return EventState::consumed();
            }
        }
        self.tv.scroll_event(ev, self.parsed.lines.len())
    }

    /// Jump the scroll position to the next/previous file section header
    /// (multi-file diffs: revision/range diffs and patch previews).
    pub fn jump_file(&mut self, forward: bool) {
        let cur = self.tv.scroll.get();
        let is_boundary = |i: usize| self.parsed.lines.get(i).is_some_and(is_file_boundary);
        let target = if forward {
            (cur + 1..self.parsed.lines.len()).find(|&i| is_boundary(i))
        } else {
            (0..cur).rev().find(|&i| is_boundary(i))
        };
        if let Some(i) = target {
            self.tv.scroll.set(i);
        }
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

/// A new section starts at an "Index: " header (svn), a "diff --git "
/// header (git-format patches), or a "Property changes on:" section
/// (property-only diff without an Index header). Only `Header`-kind lines
/// qualify: the parser classifies those prefixes at the raw-line level,
/// where content lines always carry a +/-/space marker — so a diff *of* a
/// .patch file (whose content lines themselves start with these prefixes)
/// yields no phantom section starts.
fn is_file_boundary(dl: &DiffLine) -> bool {
    dl.kind == DiffLineKind::Header
        && (dl.content.starts_with("Index: ")
            || dl.content.starts_with("diff --git ")
            || dl.content.starts_with("Property changes on:"))
}

/// Whether a line carries the +/-/space marker and a line number.
fn is_content_line(kind: DiffLineKind) -> bool {
    matches!(
        kind,
        DiffLineKind::Context | DiffLineKind::Added | DiffLineKind::Removed
    )
}

/// Prepare content lines for display: expand tabs, then make
/// whitespace-only edits visible.
///
/// Tabs are control characters the terminal buffer would drop. They are
/// expanded to spaces at the 4-column stop instead of being marked with a
/// glyph — an arrow on every indented line is visual noise, and this is how
/// a browser renders a diff. Headers keep their raw text (file headers
/// legitimately contain '\t' before "(revision N)").
///
/// Expansion alone would make a whitespace-only edit render as two
/// identical +/- lines, so such pairs get their differing blanks tinted
/// ([`DiffLine::ws_only`]): the removed side red, the added side green, and
/// cells produced by a tab in their own colour — blanks have no glyph, and
/// a TUI must not fake invisible characters with visible ones.
fn prepare_content(lines: &mut [DiffLine]) {
    let marks = ws_only_ranges(lines);
    for (i, dl) in lines.iter_mut().enumerate() {
        if !is_content_line(dl.kind) {
            continue;
        }
        let mark = marks[i];
        if mark.is_none() && !dl.content.contains('\t') {
            continue;
        }
        prepare_line(dl, mark);
    }
}

/// Expand the tabs of one content line and apply its whitespace-only mark
/// (raw char range of the differing run, if any).
fn prepare_line(dl: &mut DiffLine, mark: Option<(usize, usize)>) {
    let ui::ExpandedTabs { text, map, tabs } = ui::expand_tabs_spans(&dl.content);
    if let Some((start, end)) = mark.map(|(a, b)| (map[a], map[b])) {
        // only blanks are tinted: a run may contain text (a blank moved
        // across a word) and text must keep its readable foreground
        let chars: Vec<char> = text.chars().collect();
        let mut blanks = Vec::new();
        let mut blank_start = None;
        for i in start..end {
            if chars[i] == ' ' {
                blank_start.get_or_insert(i);
            } else if let Some(s) = blank_start.take() {
                blanks.push(byte_range(&chars, s, i));
            }
        }
        if let Some(s) = blank_start {
            blanks.push(byte_range(&chars, s, end));
        }
        let tabs = tabs
            .iter()
            .filter_map(|(s, e)| {
                let (s, e) = ((*s).max(start), (*e).min(end));
                (s < e).then(|| byte_range(&chars, s, e))
            })
            .collect();
        dl.ws_only = Some(WsMark { blanks, tabs });
    }
    dl.content = text;
}

/// Byte range of the char range `start..end` in `chars`.
fn byte_range(chars: &[char], start: usize, end: usize) -> (usize, usize) {
    let at = |idx: usize| chars[..idx].iter().map(|c| c.len_utf8()).sum::<usize>();
    (at(start), at(end))
}

/// Raw char ranges of the differing run for every line that belongs to a
/// whitespace-only change; `None` elsewhere.
///
/// A unified diff groups removed lines before added lines inside a hunk, so
/// each removed block is paired position-by-position with the added block
/// that follows it — the shape a normal "one line edited" hunk has. Pairs
/// with different lengths (a pure insertion/deletion) stay unmarked.
fn ws_only_ranges(lines: &[DiffLine]) -> Vec<Option<(usize, usize)>> {
    let mut marks = vec![None; lines.len()];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != DiffLineKind::Removed {
            i += 1;
            continue;
        }
        let removed_start = i;
        while i < lines.len() && lines[i].kind == DiffLineKind::Removed {
            i += 1;
        }
        let added_start = i;
        while i < lines.len() && lines[i].kind == DiffLineKind::Added {
            i += 1;
        }
        let pairs = (added_start - removed_start).min(i - added_start);
        for k in 0..pairs {
            let (r, a) = (removed_start + k, added_start + k);
            if let Some((removed, added)) = ws_only_difference(&lines[r].content, &lines[a].content)
            {
                marks[r] = Some(removed);
                marks[a] = Some(added);
            }
        }
    }
    marks
}

/// Differing raw char ranges of two lines whose only change is whitespace.
///
/// `None` when the lines carry different text (the change is already
/// visible) or are identical. Otherwise the common prefix and suffix are
/// kept and the ranges in between are returned, one per side: `a b` vs
/// `a  b` yields an empty range and the single added blank, so the marker
/// points at what changed instead of at the whole line.
fn ws_only_difference(removed: &str, added: &str) -> Option<((usize, usize), (usize, usize))> {
    if removed == added {
        return None;
    }
    let text = |s: &str| -> String { s.chars().filter(|c| !c.is_whitespace()).collect() };
    if text(removed) != text(added) {
        return None;
    }
    let r: Vec<char> = removed.chars().collect();
    let a: Vec<char> = added.chars().collect();
    let mut head = 0;
    while head < r.len() && head < a.len() && r[head] == a[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < r.len() - head
        && tail < a.len() - head
        && r[r.len() - 1 - tail] == a[a.len() - 1 - tail]
    {
        tail += 1;
    }
    Some(((head, r.len() - tail), (head, a.len() - tail)))
}

/// Keep only the lines that belong to a whitespace-only change, plus the
/// file and hunk headers that locate them (a bare +/- pair in a multi-file
/// diff would not say which file it came from).
fn filter_ws_only(full: &ParsedDiff) -> (ParsedDiff, Vec<usize>) {
    let mut lines = Vec::new();
    // filtered row -> row in `full`, so the view can keep its place when
    // the filter is toggled
    let mut rows = Vec::new();
    let mut section: Option<(usize, &DiffLine)> = None;
    let mut hunk: Option<(usize, &DiffLine)> = None;
    let (mut section_done, mut hunk_done) = (true, true);
    for (i, dl) in full.lines.iter().enumerate() {
        if is_file_boundary(dl) {
            section = Some((i, dl));
            section_done = false;
            hunk = None;
            hunk_done = true;
            continue;
        }
        if dl.kind == DiffLineKind::Hunk {
            hunk = Some((i, dl));
            hunk_done = false;
            continue;
        }
        if dl.ws_only.is_none() {
            continue;
        }
        // emit the file/hunk headers once, right before the first match
        for (done, header) in [(&mut section_done, section), (&mut hunk_done, hunk)] {
            if !*done && let Some((at, line)) = header {
                rows.push(at);
                lines.push(line.clone());
                *done = true;
            }
        }
        rows.push(i);
        lines.push(dl.clone());
    }
    (ParsedDiff { lines }, rows)
}

/// Split content spans at `ranges` boundaries and tint the pieces inside
/// them. Search highlighting already split the content; the tint cuts it
/// again, so both can coexist on one line.
fn tint_spans(
    spans: Vec<Span<'static>>,
    ranges: &[(usize, usize)],
    bg: Color,
) -> Vec<Span<'static>> {
    if ranges.is_empty() {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len() + ranges.len());
    let mut pos = 0;
    for span in spans {
        let text = span.content.into_owned();
        let end = pos + text.len();
        // the span's own bounds plus every range boundary that falls
        // strictly inside it (the `pos` start must survive the filter)
        let mut cuts = vec![pos, end];
        cuts.extend(
            ranges
                .iter()
                .flat_map(|(s, e)| [*s, *e])
                .filter(|c| *c > pos && *c < end),
        );
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let mut style = span.style;
            if ranges.iter().any(|(s, e)| *s <= a && b <= *e) {
                style = style.bg(bg);
            }
            out.push(Span::styled(text[a - pos..b - pos].to_string(), style));
        }
        pos = end;
    }
    out
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
    let content = match &dl.ws_only {
        Some(mark) => {
            // blanks have no glyph, so a whitespace-only change is a block
            // of background: red for the removed side, green for the added
            // one, with tab cells in their own colour on top
            let changed = if dl.kind == DiffLineKind::Removed {
                theme.ws_removed_bg
            } else {
                theme.ws_added_bg
            };
            let tinted = tint_spans(content, &mark.blanks, changed);
            tint_spans(tinted, &mark.tabs, theme.ws_tab_bg)
        }
        None => content,
    };
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
    let mut title = if view.pending {
        format!("{}  {}", view.title, crate::strings::MSG.loading_suffix)
    } else {
        view.title.clone()
    };
    if view.ws_filter {
        title.push_str("  [whitespace only]");
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border))
        .title(title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // nothing to draw: an empty diff, or a whitespace-only filter that
    // matched nothing (the title still says the filter is on)
    let notice = view.empty_reason.as_deref().or_else(|| {
        (view.ws_filter && view.parsed.lines.is_empty())
            .then_some(crate::strings::MSG.no_ws_changes)
    });
    if let Some(notice) = notice {
        f.render_widget(
            ratatui::widgets::Paragraph::new(Line::from(Span::styled(notice, theme.dim))),
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

    /// First line of `kind` in a view.
    fn line(v: &DiffView, kind: DiffLineKind) -> &DiffLine {
        v.parsed.lines.iter().find(|l| l.kind == kind).unwrap()
    }

    /// Whitespace mark of the first line of `kind` in a view.
    fn mark(v: &DiffView, kind: DiffLineKind) -> &WsMark {
        line(v, kind).ws_only.as_ref().expect("line must be marked")
    }

    /// Content of the first line of `kind` in a view.
    fn line_content(v: &DiffView, kind: DiffLineKind) -> &str {
        v.parsed
            .lines
            .iter()
            .find(|l| l.kind == kind)
            .map(|l| l.content.as_str())
            .unwrap_or("")
    }

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
    fn tabs_are_expanded_without_a_marker() {
        // the terminal buffer drops control characters, so a literal tab is
        // expanded to spaces at the tab stop — no glyph: an arrow on every
        // indented line is visual noise
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "Index: f\n@@ -1 +1,2 @@\n fn main() {\n+\tlet x = 1;\n",
        );
        let added = v
            .parsed
            .lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Added)
            .unwrap();
        assert!(!added.content.contains('\t'));
        assert_eq!(added.content, "    let x = 1;");
        assert!(!added.content.contains('\u{2192}'), "no arrow marker");
        // width math sees the expanded content: the line is indented by a
        // plain 4-space tab stop
        let t = ts::render(40, 6, |f| {
            draw_diff_block(f, Rect::new(0, 0, 40, 6), &v, &Theme::default());
        });
        let buf = t.backend().buffer();
        // row 4 = the added line: 9-column gutter, then the indent, then 'l'
        assert_eq!(buf[(10, 4)].symbol(), " ");
        assert_eq!(buf[(14, 4)].symbol(), "l");
    }

    #[test]
    fn whitespace_only_changes_are_marked_where_they_differ() {
        // blanks carry a background, never a marker glyph
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-foo\n+foo  \n");
        assert_eq!(line_content(&v, DiffLineKind::Removed), "foo");
        assert_eq!(line_content(&v, DiffLineKind::Added), "foo  ");
        assert_eq!(mark(&v, DiffLineKind::Added).blanks, vec![(3, 5)]);
        assert!(mark(&v, DiffLineKind::Removed).blanks.is_empty());

        // interior space added: only the added blank is tinted
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-a b\n+a  b\n");
        assert_eq!(line_content(&v, DiffLineKind::Added), "a  b");
        assert_eq!(mark(&v, DiffLineKind::Added).blanks, vec![(2, 3)]);
        assert!(mark(&v, DiffLineKind::Removed).blanks.is_empty());

        // ...and removed: the tint lands on the side that lost the blank
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-a  b\n+a b\n");
        assert_eq!(mark(&v, DiffLineKind::Removed).blanks, vec![(2, 3)]);
        assert!(mark(&v, DiffLineKind::Added).blanks.is_empty());

        // re-indented line: the extra leading blank is tinted
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-  foo\n+   foo\n");
        assert_eq!(mark(&v, DiffLineKind::Added).blanks, vec![(2, 3)]);

        // a blank removed from between text
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-foo( a )\n+foo(a)\n");
        assert_eq!(mark(&v, DiffLineKind::Removed).blanks, vec![(4, 5), (6, 7)]);
        assert!(mark(&v, DiffLineKind::Added).blanks.is_empty());

        // a real text change is already visible: nothing is marked
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-a b\n+c d\n");
        assert!(line(&v, DiffLineKind::Removed).ws_only.is_none());
        assert!(line(&v, DiffLineKind::Added).ws_only.is_none());

        // context lines are never marked, even with blanks
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "Index: f\n@@ -1,3 +1,3 @@\n a  b\n-x\n+y\n c  d\n",
        );
        assert!(line(&v, DiffLineKind::Context).ws_only.is_none());

        // header lines keep their raw text (file headers contain '\t')
        let mut v = DiffView::new("t");
        v.set_content("t".into(), DIFF);
        let file_header = v
            .parsed
            .lines
            .iter()
            .find(|l| l.kind == DiffLineKind::FileHeader)
            .unwrap();
        assert!(file_header.content.contains('\t'));
    }

    #[test]
    fn tint_spans_splits_and_tints() {
        let spans = vec![Span::styled("    foo".to_string(), Style::default())];
        let out = tint_spans(spans, &[(0, 4)], Color::Red);
        let got: Vec<(String, Option<Color>)> = out
            .iter()
            .map(|s| (s.content.to_string(), s.style.bg))
            .collect();
        assert_eq!(
            got,
            vec![
                ("    ".to_string(), Some(Color::Red)),
                ("foo".to_string(), None)
            ]
        );
    }

    #[test]
    fn tab_cells_get_their_own_tint() {
        // both sides expand to the same blanks; the removed side's tab
        // cells take the tab colour, the added side's spaces the "added"
        // colour, so a tab never reads as four spaces
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-\tfoo\n+    foo\n");
        assert_eq!(line_content(&v, DiffLineKind::Removed), "    foo");
        assert_eq!(line_content(&v, DiffLineKind::Added), "    foo");
        let removed = mark(&v, DiffLineKind::Removed);
        assert_eq!(removed.blanks, vec![(0, 4)]);
        assert_eq!(removed.tabs, vec![(0, 4)]);
        let added = mark(&v, DiffLineKind::Added);
        assert_eq!(added.blanks, vec![(0, 4)]);
        assert!(added.tabs.is_empty());

        let t = ts::render(40, 8, |f| {
            draw_diff_block(f, Rect::new(0, 0, 40, 8), &v, &Theme::default());
        });
        let buf = t.backend().buffer();
        let theme = Theme::default();
        // row 3 = removed line (gutter is 9 columns wide), row 4 = added
        for x in 10..14 {
            assert_eq!(buf[(x, 3)].bg, theme.ws_tab_bg, "tab cell {x}");
            assert_eq!(buf[(x, 4)].bg, theme.ws_added_bg, "space cell {x}");
        }
        assert_ne!(buf[(14, 3)].bg, theme.ws_tab_bg, "text after the tab");
    }

    #[test]
    fn removed_blanks_use_the_removed_tint() {
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-a  b\n+a b\n");
        let t = ts::render(40, 8, |f| {
            draw_diff_block(f, Rect::new(0, 0, 40, 8), &v, &Theme::default());
        });
        let buf = t.backend().buffer();
        let theme = Theme::default();
        // the removed side's blank is red, the added side has nothing tinted
        assert_eq!(buf[(12, 3)].bg, theme.ws_removed_bg);
        assert_ne!(buf[(12, 4)].bg, theme.ws_removed_bg);
    }

    #[test]
    fn whitespace_filter_keeps_only_whitespace_changes_with_context() {
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "Index: a.txt\n@@ -1,2 +1,2 @@\n-a b\n+a  b\n ctx\nIndex: b.txt\n@@ -1 +1 @@\n-x\n+y\n",
        );
        // unfiltered: everything
        assert_eq!(v.parsed.lines.len(), 9);
        v.toggle_ws_filter();
        assert!(v.ws_filter);
        // only the pair plus its file and hunk header survive
        let kept: Vec<&str> = v.parsed.lines.iter().map(|l| l.content.as_str()).collect();
        assert_eq!(
            kept,
            vec!["Index: a.txt", "@@ -1,2 +1,2 @@", "a b", "a  b"],
            "{kept:?}"
        );
        // ...and toggling back restores the whole diff
        v.toggle_ws_filter();
        assert!(!v.ws_filter);
        assert_eq!(v.parsed.lines.len(), 9);
    }

    #[test]
    fn whitespace_filter_without_matches_says_so() {
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-a\n+b\n");
        v.toggle_ws_filter();
        assert!(v.parsed.lines.is_empty());
        let t = ts::render(40, 8, |f| {
            draw_diff_block(f, Rect::new(0, 0, 40, 8), &v, &Theme::default());
        });
        let dump = ts::dump(&t);
        assert!(dump.contains("No whitespace-only changes"), "{dump}");
        // the title says why the body is empty
        assert!(dump.contains("[whitespace only]"), "{dump}");
    }

    #[test]
    fn w_key_toggles_the_whitespace_filter() {
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1,2 +1,2 @@\n-a b\n+a  b\n ctx\n");
        assert!(v.event(&ts::key(KeyCode::Char('w'))).consumed);
        assert!(v.ws_filter);
        assert!(v.event(&ts::key(KeyCode::Char('w'))).consumed);
        assert!(!v.ws_filter);
    }

    #[test]
    fn toggling_the_filter_keeps_the_reader_in_place() {
        // a long diff with one whitespace-only change in the middle: the
        // view must not jump back to the first line
        let mut text = String::from("Index: f\n@@ -1,40 +1,40 @@\n");
        for i in 0..40 {
            if i == 30 {
                text.push_str("-a b\n+a  b\n");
            } else {
                text.push_str(&format!(" ctx {i}\n"));
            }
        }
        let mut v = DiffView::new("t");
        v.set_content("t".into(), &text);
        // rows: 0 Index, 1 Hunk, 2..31 context, 32 removed, 33 added, ...
        assert_eq!(v.parsed.lines[32].kind, DiffLineKind::Removed);
        v.tv.scroll.set(25);
        v.tv.hscroll.set(5);

        v.toggle_ws_filter();
        // the first remaining line at/after the old top row (the removed
        // line at full row 32) is shown from the top
        assert_eq!(v.filter_rows, vec![0, 1, 32, 33]);
        assert_eq!(v.tv.scroll.get(), 2, "anchored on the change, not the top");
        assert_eq!(v.tv.hscroll.get(), 5, "horizontal offset is kept");

        // ...and back: the same full-diff line stays at the top
        v.toggle_ws_filter();
        assert_eq!(v.tv.scroll.get(), 32);
    }

    #[test]
    fn reload_keeps_the_filter_consistent() {
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-a b\n+a  b\n");
        v.toggle_ws_filter();
        assert_eq!(v.parsed.lines.len(), 4, "file + hunk header + the pair");
        // a reload replaces both the displayed and the parked diff
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-c d\n+e f\n");
        assert!(v.ws_filter);
        assert!(v.parsed.lines.is_empty(), "no whitespace changes left");
        v.toggle_ws_filter();
        assert_eq!(v.parsed.lines.len(), 4);
    }

    #[test]
    fn whitespace_marks_use_byte_ranges_with_multibyte_text() {
        // the mark is a *byte* range into the rendered content: a CJK char
        // before the changed blank must not shift it
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-中 a\n+中  a\n");
        assert_eq!(line_content(&v, DiffLineKind::Added), "中  a");
        // 中 = 3 bytes, then the two blanks at 3..5: the added one is 4..5
        assert_eq!(mark(&v, DiffLineKind::Added).blanks, vec![(4, 5)]);
        assert!(mark(&v, DiffLineKind::Removed).blanks.is_empty());

        // a tab after CJK text expands from column 2 to the next stop (4),
        // so it becomes two blanks, not four
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "Index: f\n@@ -1 +1 @@\n-中\tx\n+中  x\n");
        assert_eq!(line_content(&v, DiffLineKind::Removed), "中  x");
        let removed = mark(&v, DiffLineKind::Removed);
        assert_eq!(removed.blanks, vec![(3, 5)]);
        assert_eq!(removed.tabs, vec![(3, 5)]);
        assert!(mark(&v, DiffLineKind::Added).tabs.is_empty());
    }

    #[test]
    fn whitespace_marking_pairs_blocks_by_position() {
        // two changed lines: both pairs are whitespace-only
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "Index: f\n@@ -1,2 +1,2 @@\n-a b\n-c  d\n+a  b\n+c   d\n",
        );
        let marks = |kind| -> Vec<Vec<(usize, usize)>> {
            v.parsed
                .lines
                .iter()
                .filter(|l| l.kind == kind)
                .map(|l| l.ws_only.as_ref().unwrap().blanks.clone())
                .collect()
        };
        assert_eq!(marks(DiffLineKind::Removed), vec![vec![], vec![]]);
        assert_eq!(marks(DiffLineKind::Added), vec![vec![(2, 3)], vec![(3, 4)]]);
    }

    #[test]
    fn file_at_finds_the_section_above_the_line() {
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "Index: a.txt\n===\n@@ -1 +1 @@\n-a\n+b\nIndex: dir/b.txt\n===\n@@ -1 +1 @@\n-c\n+d\n",
        );
        assert_eq!(v.file_at(0).as_deref(), Some("a.txt"));
        assert_eq!(v.file_at(3).as_deref(), Some("a.txt"));
        assert_eq!(v.file_at(5).as_deref(), Some("dir/b.txt"));
        assert_eq!(v.file_at(9).as_deref(), Some("dir/b.txt"));
        // a property-only section has no Index line: its own header names
        // the file
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "Property changes on: p.txt\n___________________________________________________________________\nAdded: svn:ignore\n",
        );
        assert_eq!(v.file_at(2).as_deref(), Some("p.txt"));
        // a diff without any section header has none
        let mut v = DiffView::new("t");
        v.set_content("t".into(), "@@ -1 +1 @@\n-a\n+b\n");
        assert_eq!(v.file_at(1), None);
    }

    #[test]
    fn jump_file_moves_between_file_sections() {
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "Index: a\n@@ -1 +1 @@\n-a\n+b\nIndex: b\n@@ -1 +1 @@\n-c\n+d\nIndex: c\n@@ -1 +1 @@\n-e\n+f\n",
        );
        assert_eq!(v.tv.scroll.get(), 0);
        // ] jumps to the next file's "Index: " header
        v.event(&ts::key(KeyCode::Char(']')));
        assert_eq!(v.tv.scroll.get(), 4);
        v.event(&ts::key(KeyCode::Char(']')));
        assert_eq!(v.tv.scroll.get(), 8);
        // no further file: stays put, key still consumed
        assert!(v.event(&ts::key(KeyCode::Char(']'))).consumed);
        assert_eq!(v.tv.scroll.get(), 8);
        // scrolled into the middle of a section: [ goes to its header first
        v.tv.scroll.set(10);
        v.event(&ts::key(KeyCode::Char('[')));
        assert_eq!(v.tv.scroll.get(), 8);
        v.event(&ts::key(KeyCode::Char('[')));
        assert_eq!(v.tv.scroll.get(), 4);
        v.event(&ts::key(KeyCode::Char('[')));
        assert_eq!(v.tv.scroll.get(), 0);
        assert!(v.event(&ts::key(KeyCode::Char('['))).consumed);
        assert_eq!(v.tv.scroll.get(), 0);
    }

    #[test]
    fn added_lines_that_look_like_headers_are_not_boundaries() {
        // diffing a .patch file: its "+diff --git ..." / "+Index: ..."
        // content lines must not become jump targets
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "Index: p.patch\n@@ -0,0 +1,2 @@\n+diff --git a/x b/y\n+Index: z\n",
        );
        assert!(v.event(&ts::key(KeyCode::Char(']'))).consumed);
        assert_eq!(v.tv.scroll.get(), 0, "no real second file section");
    }

    #[test]
    fn context_lines_that_look_like_headers_are_not_boundaries() {
        // diffing a *modified* .patch file: the embedded patch's unchanged
        // header lines appear as context lines and must not be jump targets
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "Index: p.patch\n@@ -1,3 +1,3 @@\n diff --git a/x b/y\n Index: z\n-old\n+new\n",
        );
        assert!(v.event(&ts::key(KeyCode::Char(']'))).consumed);
        assert_eq!(
            v.tv.scroll.get(),
            0,
            "context header lookalikes are no sections"
        );
    }

    #[test]
    fn git_patch_headers_are_boundaries() {
        // a git-format patch preview: "diff --git " raw lines classify as
        // headers, so ] walks the patch's files
        let mut v = DiffView::new("t");
        v.set_content(
            "t".into(),
            "diff --git a/x b/y\n@@ -1 +1 @@\n-a\n+b\ndiff --git c/z d/z\n@@ -1 +1 @@\n-c\n+d\n",
        );
        v.event(&ts::key(KeyCode::Char(']')));
        assert_eq!(v.tv.scroll.get(), 4);
        v.event(&ts::key(KeyCode::Char('[')));
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
