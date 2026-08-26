//! Fullscreen diff popup (file diff or revision diff).

use super::super::components::{
    Context, DrawableComponent, EventState,
    diff_view::{DiffView, draw_diff_block},
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
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::ClosePopup) || key_match(k, KeyAction::Quit) {
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        }
        let consumed = self.view.event(ev);
        if consumed.consumed {
            return Ok(EventState::consumed());
        }
        // any other key closes the popup for convenience
        self.ctx.queue.push(InternalEvent::ClosePopup);
        Ok(EventState::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::InternalEvent;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    #[test]
    fn scroll_close_and_any_key() {
        let q = crate::queue::Queue::new();
        let c = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let content = "Index: a\n===\n@@ -1 +1 @@\n-a\n+b\n";
        let mut p = DiffPopup::new(&c, "a".to_string(), content);
        assert_eq!(p.view.parsed.lines.len(), 5);
        p.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(p.view.scroll.get(), 1);
        // Esc closes
        p.event(&ts::key(crossterm::event::KeyCode::Esc)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        // any other key closes too
        let mut p2 = DiffPopup::new(&c, "a".to_string(), content);
        p2.event(&ts::key(crossterm::event::KeyCode::Char('x')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
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
