//! Scrollable diff rendering, shared between the status tab diff pane and
//! the fullscreen diff popup.

use super::EventState;
use crate::keys::{KeyAction, key_match};
use crate::svn::models::{DiffLine, DiffLineKind, ParsedDiff};
use crate::svn::parser::{parse_diff, parse_new_file_content};
use crate::ui::{self, style::Theme};
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use std::cell::Cell;

/// A scrollable view of a parsed diff.
pub struct DiffView {
    pub title: String,
    pub parsed: ParsedDiff,
    pub scroll: Cell<usize>,
    pub pending: bool,
    /// Set when there is nothing to show
    pub empty_reason: Option<String>,
    pub focused: bool,
}

impl DiffView {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_string(),
            parsed: ParsedDiff::default(),
            scroll: Cell::new(0),
            pending: true,
            empty_reason: None,
            focused: true,
        }
    }

    pub fn set_loading(&mut self, title: String) {
        self.title = title;
        self.pending = true;
        self.empty_reason = None;
    }

    /// Show a placeholder message instead of a diff (e.g. "select a file").
    pub fn set_hint(&mut self, title: String, reason: String) {
        self.title = title;
        self.pending = false;
        self.empty_reason = Some(reason);
        self.parsed = ParsedDiff::default();
    }

    /// Set raw diff text (or raw file content for unversioned files).
    pub fn set_content(&mut self, title: String, content: &str) {
        self.title = title;
        self.pending = false;
        self.scroll.set(0);
        let trimmed = content.trim();
        if trimmed.is_empty() {
            self.empty_reason = Some("No textual diff".to_string());
            self.parsed = ParsedDiff::default();
        } else if trimmed.starts_with("Index:")
            || trimmed.contains("@@")
            || trimmed.starts_with("Property changes on:")
        {
            self.parsed = parse_diff(content);
            self.empty_reason = None;
        } else {
            self.parsed = parse_new_file_content(content);
            self.empty_reason = None;
        }
    }

    pub fn event(&mut self, ev: &Event) -> EventState {
        let Event::Key(k) = ev else {
            return EventState::not_consumed();
        };
        let len = self.parsed.lines.len();
        let mut scroll = self.scroll.get();
        if key_match(k, KeyAction::MoveDown) || key_match(k, KeyAction::PageDown) {
            scroll = scroll.saturating_add(if key_match(k, KeyAction::PageDown) {
                20
            } else {
                1
            });
        } else if key_match(k, KeyAction::MoveUp) || key_match(k, KeyAction::PageUp) {
            scroll = scroll.saturating_sub(if key_match(k, KeyAction::PageUp) {
                20
            } else {
                1
            });
        } else if key_match(k, KeyAction::Home) {
            scroll = 0;
        } else if key_match(k, KeyAction::End) {
            scroll = len;
        } else {
            return EventState::not_consumed();
        }
        self.scroll.set(scroll.min(len));
        EventState::consumed()
    }

    /// Build the styled lines for this diff.
    pub fn lines(&self, theme: &Theme) -> Vec<Line<'static>> {
        let mut out = Vec::with_capacity(self.parsed.lines.len());
        for dl in &self.parsed.lines {
            out.push(diff_line(dl, theme));
        }
        out
    }
}

/// Build a single styled diff line with line numbers.
pub fn diff_line(dl: &DiffLine, theme: &Theme) -> Line<'static> {
    let (kind_style, need_numbers) = match dl.kind {
        DiffLineKind::Header => (theme.diff_header, false),
        DiffLineKind::FileHeader => (theme.diff_file_header, false),
        DiffLineKind::Hunk => (theme.diff_hunk, false),
        DiffLineKind::Note => (theme.diff_note, false),
        DiffLineKind::Context => (theme.text, true),
        DiffLineKind::Added => (theme.diff_added, true),
        DiffLineKind::Removed => (theme.diff_removed, true),
    };

    if !need_numbers {
        return Line::from(Span::styled(dl.content.clone(), kind_style));
    }

    let num = |n: Option<u64>| -> String {
        match n {
            Some(n) => format!("{n:>3}"),
            None => "   ".to_string(),
        }
    };
    let spans = vec![
        Span::styled(num(dl.old), theme.diff_line_number),
        Span::raw(" "),
        Span::styled(num(dl.new), theme.diff_line_number),
        Span::styled("│ ", theme.diff_line_number),
        Span::styled(dl.content.clone(), kind_style),
    ];
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
        format!("{}  (loading…)", view.title)
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
            ratatui::widgets::Paragraph::new(Line::from(Span::styled("Loading...", theme.dim))),
            inner,
        );
        return;
    }

    let lines = view.lines(theme);
    let scroll = ui::clamp_scroll(view.scroll.get(), lines.len(), inner.height as usize);
    view.scroll.set(scroll);
    ui::render_lines(f, inner, &lines, scroll, &[]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

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
        let lines = v.lines(&theme);
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
    fn scrolling_clamps() {
        let mut v = DiffView::new("t");
        v.set_content("t".into(), DIFF);
        v.event(&ts::key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(v.scroll.get(), 1);
        v.event(&ts::key(crossterm::event::KeyCode::Char('G')));
        assert_eq!(v.scroll.get(), 8);
        v.event(&ts::key(crossterm::event::KeyCode::Char('g')));
        assert_eq!(v.scroll.get(), 0);
        v.event(&ts::key(crossterm::event::KeyCode::PageDown));
        assert_eq!(v.scroll.get(), 8); // bounded by content
        v.event(&ts::key(crossterm::event::KeyCode::PageUp));
        assert_eq!(v.scroll.get(), 0);
        // unknown key not consumed
        let state = v.event(&ts::key(crossterm::event::KeyCode::Char('x')));
        assert!(!state.consumed);
        // scrolling is bounded by the content length
        v.event(&ts::key(crossterm::event::KeyCode::Char('G')));
        v.event(&ts::key(crossterm::event::KeyCode::Char('j')));
        assert_eq!(v.scroll.get(), 8);
        let _ = ts::render(50, 4, |f| {
            draw_diff_block(f, Rect::new(0, 0, 50, 4), &v, &Theme::default());
        });
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
