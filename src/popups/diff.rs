//! Fullscreen diff popup (file diff or revision diff).

use super::super::components::{
    Context, DrawableComponent, EventState,
    diff_view::{DiffView, draw_diff_block},
    text_search::EscAction,
};
use crate::keys::{KeyAction, key_match};
use crate::queue::InternalEvent;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Clear;

pub struct DiffPopup {
    ctx: Context,
    pub view: DiffView,
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
        assert!(has_bg(ratatui::style::Color::Yellow));
        assert!(has_bg(ratatui::style::Color::Magenta));
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
