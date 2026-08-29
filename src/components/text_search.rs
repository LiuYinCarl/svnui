//! Vim-like incremental search state shared by scrollable text popups
//! (fullscreen diff, blame). The state is view-agnostic: a view feeds it
//! its plain text lines, gets back match positions, scrolls to the current
//! match itself and renders the highlights + footer line.
//!
//! Key semantics (documented for both popups):
//! - `/` enters input mode with a *fresh* pattern (the previous pattern is
//!   not reused); typing/paste/backspace updates the pattern live and the
//!   view scrolls to the first match at or after the current scroll offset.
//! - `Enter` leaves input mode but keeps the highlights; `n`/`N` then cycle
//!   the current match (wrapping) and update the `[x/y]` counter.
//! - the Esc three-state lives in [`TextSearch::esc`]: cancel the input,
//!   clear confirmed highlights, or — with no search state — signal the
//!   popup to close (the popup owns the close action).

use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::Span;

/// One match: the `start..end` byte range within `line`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Result of feeding one event to [`TextSearch::input_event`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputOutcome {
    /// The pattern changed; the caller should recompute matches.
    Changed,
    /// Enter: leave input mode, keep pattern and highlights.
    Confirmed,
    /// Esc: leave input mode, pattern and highlights cleared.
    Cancelled,
    /// Event ignored (e.g. cursor keys); input mode continues.
    Ignored,
}

/// What an Esc keypress did, given the search state (the three-state Esc
/// shared by the fullscreen diff popup and the blame popup).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscAction {
    /// In input mode: the input was cancelled (pattern + highlights dropped).
    CancelInput,
    /// Confirmed highlights were showing and are now cleared.
    ClearHighlights,
    /// No search state at all: the caller should close the popup.
    ClosePopup,
}

/// Incremental-search state: pattern, matches and the current-match cursor.
#[derive(Default)]
pub struct TextSearch {
    pattern: String,
    input_mode: bool,
    /// All matches, sorted by (line, start)
    matches: Vec<Match>,
    /// Index into `matches` of the current match
    current: Option<usize>,
}

impl TextSearch {
    pub fn new() -> Self {
        Self::default()
    }

    /// True while the user is typing the pattern.
    pub fn is_input_mode(&self) -> bool {
        self.input_mode
    }

    /// True while the footer should be shown (typing or highlights active).
    pub fn is_active(&self) -> bool {
        self.input_mode || !self.pattern.is_empty()
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    /// Start a fresh search: enter input mode with an empty pattern.
    pub fn start_input(&mut self) {
        self.input_mode = true;
        self.pattern.clear();
        self.matches.clear();
        self.current = None;
    }

    /// Cancel the search: leave input mode, drop pattern and highlights.
    pub fn cancel(&mut self) {
        self.clear();
    }

    /// The Esc three-state shared by the scrollable text popups: cancel an
    /// in-progress input, clear confirmed highlights, or — with no search
    /// state at all — signal that the popup itself should close.
    pub fn esc(&mut self) -> EscAction {
        if self.is_input_mode() {
            self.cancel();
            EscAction::CancelInput
        } else if self.is_active() {
            self.clear();
            EscAction::ClearHighlights
        } else {
            EscAction::ClosePopup
        }
    }

    /// Drop pattern, matches and input mode.
    pub fn clear(&mut self) {
        self.input_mode = false;
        self.pattern.clear();
        self.matches.clear();
        self.current = None;
    }

    /// Handle one event while in input mode (no-op otherwise).
    pub fn input_event(&mut self, ev: &Event) -> InputOutcome {
        if !self.input_mode {
            return InputOutcome::Ignored;
        }
        match ev {
            // bracketed paste: pasted text arrives as one Event::Paste
            Event::Paste(text) => {
                self.pattern.push_str(text);
                InputOutcome::Changed
            }
            Event::Key(k) => match k.code {
                KeyCode::Enter => {
                    self.input_mode = false;
                    InputOutcome::Confirmed
                }
                KeyCode::Esc => {
                    self.cancel();
                    InputOutcome::Cancelled
                }
                KeyCode::Backspace => {
                    self.pattern.pop();
                    InputOutcome::Changed
                }
                // no ctrl/alt combos: macOS Option+letter produces chars
                // like ´ that must not pollute the pattern
                KeyCode::Char(c)
                    if !k
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    self.pattern.push(c);
                    InputOutcome::Changed
                }
                _ => InputOutcome::Ignored,
            },
            _ => InputOutcome::Ignored,
        }
    }

    /// Recompute all matches against `lines`. The current match becomes the
    /// first one at or after `from_line` (wrapping to the very first match),
    /// so live typing scrolls to the nearest match below the scroll offset.
    pub fn recompute(&mut self, lines: &[&str], from_line: usize) {
        self.matches.clear();
        self.current = None;
        if self.pattern.is_empty() {
            return;
        }
        for (line, text) in lines.iter().enumerate() {
            for (start, _) in text.match_indices(&self.pattern) {
                self.matches.push(Match {
                    line,
                    start,
                    end: start + self.pattern.len(),
                });
            }
        }
        if !self.matches.is_empty() {
            self.current = Some(
                self.matches
                    .iter()
                    .position(|m| m.line >= from_line)
                    .unwrap_or(0),
            );
        }
    }

    /// Advance to the next match (wrapping); returns its line.
    pub fn next_match(&mut self) -> Option<usize> {
        self.step(1)
    }

    /// Retreat to the previous match (wrapping); returns its line.
    pub fn prev_match(&mut self) -> Option<usize> {
        self.step(-1)
    }

    fn step(&mut self, dir: isize) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        let n = self.matches.len() as isize;
        let cur = self.current.unwrap_or(0) as isize;
        let next = (cur + dir).rem_euclid(n) as usize;
        self.current = Some(next);
        Some(self.matches[next].line)
    }

    /// Line of the current match, if any.
    pub fn current_match_line(&self) -> Option<usize> {
        self.current.map(|i| self.matches[i].line)
    }

    /// Byte ranges of all matches on `line`, plus the index within that
    /// slice of the current match (if it is on this line) for distinct
    /// styling.
    pub fn line_ranges(&self, line: usize) -> (Vec<(usize, usize)>, Option<usize>) {
        let first = self.matches.partition_point(|m| m.line < line);
        let count = self.matches[first..].partition_point(|m| m.line == line);
        let ranges = self.matches[first..first + count]
            .iter()
            .map(|m| (m.start, m.end))
            .collect();
        let current = self
            .current
            .filter(|c| (first..first + count).contains(c))
            .map(|c| c - first);
        (ranges, current)
    }

    /// Footer text: `/pattern  [x/y]`, or `[no match]` when the pattern
    /// matches nothing.
    pub fn status_text(&self) -> String {
        if self.pattern.is_empty() {
            return "/".to_string();
        }
        let counter = if self.matches.is_empty() {
            "[no match]".to_string()
        } else {
            format!(
                "[{}/{}]",
                self.current.map_or(0, |i| i + 1),
                self.matches.len()
            )
        };
        format!("/{}  {}", self.pattern, counter)
    }
}

/// Split `text` into spans: each `(start, end)` byte range in `ranges`
/// (sorted, non-overlapping, as produced by [`TextSearch::line_ranges`])
/// gets the `hit` style — `current_hit` for the range at index `current` —
/// and everything in between gets `base`.
pub fn highlight_spans(
    text: &str,
    base: Style,
    ranges: &[(usize, usize)],
    current: Option<usize>,
    hit: Style,
    current_hit: Style,
) -> Vec<Span<'static>> {
    if ranges.is_empty() {
        return vec![Span::styled(text.to_string(), base)];
    }
    let mut spans = Vec::with_capacity(ranges.len() * 2 + 1);
    let mut pos = 0;
    for (i, &(s, e)) in ranges.iter().enumerate() {
        if s > pos {
            spans.push(Span::styled(text[pos..s].to_string(), base));
        }
        let style = if current == Some(i) { current_hit } else { hit };
        spans.push(Span::styled(text[s..e].to_string(), style));
        pos = e;
    }
    if pos < text.len() {
        spans.push(Span::styled(text[pos..].to_string(), base));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;
    use crossterm::event::KeyCode;

    fn lines() -> Vec<&'static str> {
        vec!["foo bar", "nothing here", "bar foo bar", "last foo"]
    }

    fn search_with(pattern: &str, from: usize) -> TextSearch {
        let mut s = TextSearch::new();
        s.start_input();
        for c in pattern.chars() {
            s.input_event(&ts::key(KeyCode::Char(c)));
        }
        s.recompute(&lines(), from);
        s
    }

    #[test]
    fn input_events_update_pattern_incrementally() {
        let mut s = TextSearch::new();
        // not in input mode: ignored
        assert_eq!(
            s.input_event(&ts::key(KeyCode::Char('a'))),
            InputOutcome::Ignored
        );
        s.start_input();
        assert!(s.is_input_mode());
        assert!(s.is_active());
        assert_eq!(
            s.input_event(&ts::key(KeyCode::Char('f'))),
            InputOutcome::Changed
        );
        assert_eq!(
            s.input_event(&Event::Paste("oo".to_string())),
            InputOutcome::Changed
        );
        assert_eq!(s.pattern(), "foo");
        assert_eq!(
            s.input_event(&ts::key(KeyCode::Backspace)),
            InputOutcome::Changed
        );
        assert_eq!(s.pattern(), "fo");
        // ctrl/alt-modified chars and cursor keys are ignored
        let ctrl_a = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(s.input_event(&ctrl_a), InputOutcome::Ignored);
        // macOS Option+e produces '´' with the ALT modifier set
        let alt_e = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('´'),
            KeyModifiers::ALT,
        ));
        assert_eq!(s.input_event(&alt_e), InputOutcome::Ignored);
        assert_eq!(
            s.input_event(&ts::key(KeyCode::Down)),
            InputOutcome::Ignored
        );
        assert_eq!(s.pattern(), "fo");
    }

    #[test]
    fn recompute_finds_all_matches_and_picks_nearest() {
        let s = search_with("foo", 0);
        assert_eq!(s.match_count(), 3);
        assert_eq!(s.current_match_line(), Some(0));
        // from_line picks the first match at or after it
        let s2 = search_with("foo", 1);
        assert_eq!(s2.current_match_line(), Some(2));
        // past the last match: wraps to the first
        let s3 = search_with("foo", 4);
        assert_eq!(s3.current_match_line(), Some(0));
        // multiple matches on one line (line 2 has two "bar"s)
        let s4 = search_with("bar", 0);
        let (ranges, cur) = s4.line_ranges(2);
        assert_eq!(ranges.len(), 2);
        assert_eq!(cur, None);
    }

    #[test]
    fn next_prev_wrap_around() {
        let mut s = search_with("foo", 0);
        assert_eq!(s.next_match(), Some(2));
        assert_eq!(s.next_match(), Some(3));
        assert_eq!(s.next_match(), Some(0)); // wrapped
        assert_eq!(s.prev_match(), Some(3)); // wrapped back
        assert_eq!(s.prev_match(), Some(2));
        // two matches on one line are stepped through individually
        let mut s = search_with("bar", 0);
        assert_eq!(s.next_match(), Some(2));
        assert_eq!(s.next_match(), Some(2)); // second match on line 2
        assert_eq!(s.next_match(), Some(0)); // wrapped
    }

    #[test]
    fn status_text_formats() {
        let mut s = TextSearch::new();
        s.start_input();
        assert_eq!(s.status_text(), "/");
        let mut s = search_with("foo", 0);
        assert_eq!(s.status_text(), "/foo  [1/3]");
        s.next_match();
        assert_eq!(s.status_text(), "/foo  [2/3]");
        let s2 = search_with("zzz", 0);
        assert_eq!(s2.status_text(), "/zzz  [no match]");
        assert_eq!(s2.match_count(), 0);
        assert_eq!(s2.current_match_line(), None);
    }

    #[test]
    fn confirm_keeps_highlights_esc_clears() {
        let mut s = search_with("foo", 0);
        assert_eq!(
            s.input_event(&ts::key(KeyCode::Enter)),
            InputOutcome::Confirmed
        );
        assert!(!s.is_input_mode());
        assert!(s.is_active()); // highlights stay
        assert_eq!(s.match_count(), 3);
        // Esc now (outside input mode) is handled by the view via clear()
        s.clear();
        assert!(!s.is_active());
        assert_eq!(s.match_count(), 0);

        // Esc during input cancels everything
        let mut s2 = search_with("foo", 0);
        assert_eq!(
            s2.input_event(&ts::key(KeyCode::Esc)),
            InputOutcome::Cancelled
        );
        assert!(!s2.is_input_mode());
        assert!(!s2.is_active());
        assert_eq!(s2.match_count(), 0);
    }

    #[test]
    fn esc_three_states() {
        // input mode: cancel
        let mut s = search_with("foo", 0);
        assert!(s.is_input_mode());
        assert_eq!(s.esc(), EscAction::CancelInput);
        assert!(!s.is_active());
        // confirmed highlights: clear, popup stays open
        let mut s = search_with("foo", 0);
        s.input_event(&ts::key(KeyCode::Enter));
        assert!(s.is_active());
        assert_eq!(s.esc(), EscAction::ClearHighlights);
        assert!(!s.is_active());
        // no search state: the caller closes the popup
        assert_eq!(s.esc(), EscAction::ClosePopup);
    }

    #[test]
    fn line_ranges_reports_current_local_index() {
        let mut s = search_with("bar", 0);
        // matches: line 0 [4..7], line 2 [0..3] and [8..11]
        let (ranges, cur) = s.line_ranges(2);
        assert_eq!(ranges, vec![(0, 3), (8, 11)]);
        assert_eq!(cur, None);
        s.next_match(); // now on line 2, first range
        let (ranges, cur) = s.line_ranges(2);
        assert_eq!(ranges.len(), 2);
        assert_eq!(cur, Some(0));
        s.next_match();
        let (_, cur) = s.line_ranges(2);
        assert_eq!(cur, Some(1));
        // a line without matches
        let (ranges, cur) = s.line_ranges(1);
        assert!(ranges.is_empty());
        assert_eq!(cur, None);
    }

    #[test]
    fn highlight_spans_splits_text() {
        let base = Style::default();
        let hit = Style::default().bg(ratatui::style::Color::Yellow);
        let cur = Style::default().bg(ratatui::style::Color::Magenta);
        // no ranges: single span
        let spans = highlight_spans("abc", base, &[], None, hit, cur);
        assert_eq!(spans.len(), 1);
        // ranges at start/middle/end with a current marker
        let spans = highlight_spans("foo bar foo", base, &[(0, 3), (8, 11)], Some(1), hit, cur);
        let texts: Vec<&str> = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(texts, vec!["foo", " bar ", "foo"]);
        assert_eq!(spans[0].style, hit);
        assert_eq!(spans[2].style, cur);
        // trailing text after the last range is kept
        let spans = highlight_spans("foo!", base, &[(0, 3)], Some(0), hit, cur);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].content.as_ref(), "!");
    }
}
