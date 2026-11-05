//! Fullscreen diff popup (file diff or revision diff).

use super::super::components::{
    Context, DrawableComponent, EventState,
    diff_view::{DiffView, draw_diff_block},
    text_search::EscAction,
};
use crate::keys::{KeyAction, key_match};
use crate::queue::InternalEvent;
use crate::svn::models::PreparedDiff;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Clear;
use std::path::PathBuf;

/// What a fullscreen diff was loaded from. F5 in the popup re-runs this
/// same request (svn diff / svn diff -c / svn diff -r / patch re-read).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffSource {
    /// Working-copy diff of one file (`svn diff`)
    File(String),
    /// A revision diff, optionally limited to one file (`svn diff -c`)
    Revision(u64, Option<String>),
    /// Combined diff of revisions `from..=to` (`svn diff -r`)
    Range(u64, u64),
    /// A patch file shown as a diff (re-read from disk)
    Patch(PathBuf),
}

pub struct DiffPopup {
    ctx: Context,
    pub view: DiffView,
    /// Request that produced the content; `None` for static content,
    /// which F5 cannot reload
    pub source: Option<DiffSource>,
}

impl DiffPopup {
    pub fn new(ctx: &Context, title: String, content: &str) -> Self {
        let mut view = DiffView::new(&title);
        view.set_content(title, content);
        view.focused = true;
        view.search_enabled = true;
        Self {
            ctx: ctx.clone(),
            view,
            source: None,
        }
    }

    /// Build the popup from a diff already parsed on the svn worker
    /// thread (repository diffs); `new` stays for UI-side parsing (patch
    /// previews, tests).
    pub fn new_prepared(ctx: &Context, title: String, prepared: PreparedDiff) -> Self {
        let mut view = DiffView::new(&title);
        view.set_prepared(title, prepared);
        view.focused = true;
        view.search_enabled = true;
        Self {
            ctx: ctx.clone(),
            view,
            source: None,
        }
    }

    /// Install a freshly loaded diff in place (F5). The scroll position,
    /// the horizontal offset and an active search survive; the header is
    /// replaced.
    pub fn reload(&mut self, prepared: PreparedDiff, header: Vec<String>) {
        let scroll = self.view.tv.scroll.get();
        let hscroll = self.view.tv.hscroll.get();
        // `set_prepared` resets the text view, which would drop the search
        let search = std::mem::take(&mut self.view.tv.search);
        let title = self.view.title.clone();
        self.view.set_prepared(title, prepared);
        self.view.set_header(header);
        self.view
            .tv
            .scroll
            .set(scroll.min(self.view.parsed.lines.len()));
        self.view.tv.hscroll.set(hscroll);
        self.view.tv.search = search;
        if self.view.tv.search.is_active() {
            let lines: Vec<&str> = self
                .view
                .parsed
                .lines
                .iter()
                .map(|l| l.content.as_str())
                .collect();
            self.view.tv.search.recompute(&lines, scroll);
        }
    }
}

impl DrawableComponent for DiffPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        f.render_widget(Clear, area);
        draw_diff_block(f, area, &self.view, &self.ctx.theme);
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        // Esc: cancel search input / clear search highlights first; only
        // with no active search does it close the popup.
        if let Event::Key(k) = ev
            && key_match(k, KeyAction::ClosePopup)
        {
            if self.view.tv.search.esc() == EscAction::ClosePopup {
                self.ctx.queue.push(InternalEvent::ClosePopup);
            }
            return Ok(EventState::consumed());
        }
        // search input / `/` / n / N
        if let Some(state) = self.view.search_event(ev) {
            return Ok(state);
        }
        if let Event::Key(k) = ev
            && key_match(k, KeyAction::Quit)
        {
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        }
        // F5 / R: re-run the request behind this diff. Checked after the
        // search input, so 'R' typed into a search pattern stays text.
        if let Event::Key(k) = ev
            && key_match(k, KeyAction::Refresh)
        {
            if self.source.is_some() {
                self.ctx.queue.push(InternalEvent::RefreshDiff);
            }
            return Ok(EventState::consumed());
        }
        let consumed = self.view.event(ev);
        if consumed.consumed {
            return Ok(EventState::consumed());
        }
        // any other key is ignored, but consumed so it cannot leak through
        // to the tab underneath (it used to close the popup)
        Ok(EventState::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::InternalEvent;
    use crate::test_support as ts;
    use crate::ui::style::Theme;
    use crossterm::event::KeyCode;

    fn popup(content: &str) -> (DiffPopup, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let c = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        (DiffPopup::new(&c, "a".to_string(), content), q)
    }

    /// A diff with the needle "needle" on parsed lines 8 and 10, far enough
    /// down that a match forces scrolling.
    const SEARCH_DIFF: &str = "\
Index: f
===================================================================
--- f\t(revision 1)
+++ f\t(working copy)
@@ -1,5 +1,6 @@
 line one
 line two
 line three
+needle one
 line four
+needle two
 line five
";

    #[test]
    fn scroll_close_and_any_key() {
        let content = "Index: a\n===\n@@ -1 +1 @@\n-a\n+b\n";
        let (mut p, q) = popup(content);
        assert_eq!(p.view.parsed.lines.len(), 5);
        p.event(&ts::key(KeyCode::Char('j'))).unwrap();
        assert_eq!(p.view.tv.scroll.get(), 1);
        // Esc closes
        p.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        // q closes too
        let (mut p2, q2) = popup(content);
        p2.event(&ts::key(KeyCode::Char('q'))).unwrap();
        assert!(matches!(q2.pop(), Some(InternalEvent::ClosePopup)));
        // any other key is ignored now (no longer closes the popup)
        let (mut p3, q3) = popup(content);
        p3.event(&ts::key(KeyCode::Char('x'))).unwrap();
        assert!(q3.pop().is_none());
    }

    #[test]
    fn incremental_search_highlights_and_scrolls() {
        let (mut p, q) = popup(SEARCH_DIFF);
        // '/' enters input mode with a fresh pattern
        p.event(&ts::key(KeyCode::Char('/'))).unwrap();
        assert!(p.view.tv.search.is_input_mode());
        // typing updates the pattern live; 'q' is pattern text here
        for c in "need".chars() {
            p.event(&ts::key(KeyCode::Char(c))).unwrap();
        }
        assert_eq!(p.view.tv.search.pattern(), "need");
        assert_eq!(p.view.tv.search.match_count(), 2);
        // scrolled to the first match (line index 8 in parsed.lines)
        assert_eq!(p.view.tv.scroll.get(), 8);
        let t = ts::render(60, 8, |f| {
            p.draw(f, Rect::new(0, 0, 60, 8)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("/need  [1/2]"), "{s}");
        assert!(s.contains("needle one"), "{s}");
        // match cells carry the search-hit backgrounds (current vs other)
        let buf = t.backend().buffer();
        let has_bg =
            |color: ratatui::style::Color| (0..8).any(|y| (0..60).any(|x| buf[(x, y)].bg == color));
        let theme = Theme::default();
        assert!(has_bg(theme.search_hit.bg.unwrap()));
        assert!(has_bg(theme.search_hit_current.bg.unwrap()));
        assert!(q.pop().is_none());
    }

    #[test]
    fn enter_keeps_highlights_n_and_big_n_cycle_esc_clears_then_closes() {
        let (mut p, q) = popup(SEARCH_DIFF);
        p.event(&ts::key(KeyCode::Char('/'))).unwrap();
        for c in "needle".chars() {
            p.event(&ts::key(KeyCode::Char(c))).unwrap();
        }
        // Enter confirms: input mode off, highlights stay
        p.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(!p.view.tv.search.is_input_mode());
        assert!(p.view.tv.search.is_active());
        assert_eq!(p.view.tv.scroll.get(), 8);
        // n jumps to the next match and updates the counter
        p.event(&ts::key(KeyCode::Char('n'))).unwrap();
        assert_eq!(p.view.tv.scroll.get(), 10);
        assert_eq!(p.view.tv.search.status_text(), "/needle  [2/2]");
        // n wraps to the first match, N back again
        p.event(&ts::key(KeyCode::Char('n'))).unwrap();
        assert_eq!(p.view.tv.search.status_text(), "/needle  [1/2]");
        p.event(&ts::key(KeyCode::Char('N'))).unwrap();
        assert_eq!(p.view.tv.search.status_text(), "/needle  [2/2]");
        // scrolling still works with highlights active
        p.event(&ts::key(KeyCode::Char('j'))).unwrap();
        // first Esc clears the highlights, popup stays open
        p.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(!p.view.tv.search.is_active());
        assert!(q.pop().is_none());
        // second Esc closes
        p.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }

    #[test]
    fn esc_in_input_mode_cancels_search_only() {
        let (mut p, q) = popup(SEARCH_DIFF);
        p.event(&ts::key(KeyCode::Char('/'))).unwrap();
        for c in "needle".chars() {
            p.event(&ts::key(KeyCode::Char(c))).unwrap();
        }
        p.event(&ts::key(KeyCode::Esc)).unwrap();
        // search cancelled, popup still open
        assert!(!p.view.tv.search.is_input_mode());
        assert!(!p.view.tv.search.is_active());
        assert!(q.pop().is_none());
        // paste works in input mode
        p.event(&ts::key(KeyCode::Char('/'))).unwrap();
        p.event(&Event::Paste("zzz".to_string())).unwrap();
        assert_eq!(p.view.tv.search.pattern(), "zzz");
        let t = ts::render(60, 8, |f| {
            p.draw(f, Rect::new(0, 0, 60, 8)).unwrap();
        });
        assert!(ts::dump(&t).contains("/zzz  [no match]"));
    }

    #[test]
    fn f5_requests_reload_only_with_a_source() {
        let (mut p, q) = popup(SEARCH_DIFF);
        // static content (no source) has nothing to re-run
        p.event(&ts::key(KeyCode::F(5))).unwrap();
        assert!(q.pop().is_none());
        p.source = Some(DiffSource::File("f".into()));
        p.event(&ts::key(KeyCode::F(5))).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::RefreshDiff)));
        // 'R' reloads as well, but typed into a search pattern it is text
        p.event(&ts::key(KeyCode::Char('/'))).unwrap();
        p.event(&ts::key(KeyCode::Char('R'))).unwrap();
        assert_eq!(p.view.tv.search.pattern(), "R");
        assert!(q.pop().is_none());
    }

    #[test]
    fn reload_keeps_scroll_and_search() {
        let (mut p, _q) = popup(SEARCH_DIFF);
        p.event(&ts::key(KeyCode::Char('/'))).unwrap();
        for c in "needle".chars() {
            p.event(&ts::key(KeyCode::Char(c))).unwrap();
        }
        p.event(&ts::key(KeyCode::Enter)).unwrap();
        assert_eq!(p.view.tv.scroll.get(), 8);
        assert_eq!(p.view.tv.search.status_text(), "/needle  [1/2]");
        // the reloaded content gains a third match below the scroll offset
        let fresh = format!("{SEARCH_DIFF}+needle three\n");
        p.reload(
            crate::svn::parser::parse_diff_content(&fresh),
            vec!["hdr".into()],
        );
        assert_eq!(p.view.tv.scroll.get(), 8);
        assert_eq!(p.view.header(), &["hdr"]);
        assert!(p.view.tv.search.is_active());
        assert_eq!(p.view.tv.search.status_text(), "/needle  [1/3]");
    }

    #[test]
    fn draw_content() {
        let q = crate::queue::Queue::new();
        let c = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let p = DiffPopup::new(
            &c,
            "src/main.rs".to_string(),
            "Index: src/main.rs\n===\n@@ -1 +1 @@\n-old\n+new\n",
        );
        let t = ts::render(80, 10, |f| {
            p.draw(f, Rect::new(0, 0, 80, 10)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("src/main.rs"), "{s}");
        assert!(s.contains("old"), "{s}");
        assert!(s.contains("new"), "{s}");
    }
}
