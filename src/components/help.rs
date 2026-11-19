//! Help popup: lists all keybindings.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, all_binding_groups, key_match};
use crate::queue::InternalEvent;
use crate::ui;
use crossterm::event::{Event, KeyCode};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear};
use std::cell::Cell;

pub struct HelpPopup {
    ctx: Context,
    scroll: Cell<usize>,
}

impl HelpPopup {
    pub fn new(ctx: &Context) -> Self {
        Self {
            ctx: ctx.clone(),
            scroll: Cell::new(0),
        }
    }
}

/// Wrap a help description to `width` columns. Words are kept whole
/// where possible; a single word longer than the width is hard-split so
/// nothing is silently clipped. Returns at least one line.
fn wrap_description(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if current.is_empty() {
            if word_len > width {
                let mut rest = word;
                while rest.chars().count() > width {
                    let split = rest
                        .char_indices()
                        .nth(width)
                        .map(|(i, _)| i)
                        .unwrap_or(rest.len());
                    lines.push(rest[..split].to_string());
                    rest = &rest[split..];
                }
                current.push_str(rest);
            } else {
                current.push_str(word);
            }
        } else if current.chars().count() + 1 + word_len <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

impl DrawableComponent for HelpPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        // clear the cells behind the popup so the underlying tab content
        // does not bleed through
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(crate::strings::TITLE.help);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(
            "svnui — an SVN TUI inspired by gitui",
            theme.title_focused,
        )));
        lines.push(Line::from(""));
        for group in all_binding_groups() {
            lines.push(Line::from(Span::styled(group.title, theme.title_focused)));
            // The key column is 20 cells wide (two spaces + 18); wrap the
            // description into what is left instead of letting the buffer
            // clip it (at 80 cols a single line only fits ~38 chars).
            let desc_width = (inner.width as usize).saturating_sub(20);
            for b in group.bindings {
                for (i, part) in wrap_description(&b.description, desc_width)
                    .into_iter()
                    .enumerate()
                {
                    let prefix = if i == 0 {
                        format!("  {: <18}", b.keys)
                    } else {
                        " ".repeat(20)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(prefix, theme.info),
                        Span::styled(part, theme.text),
                    ]));
                }
            }
            lines.push(Line::from(""));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Press q or Esc to close",
            theme.dim,
        )));

        let scroll = ui::clamp_scroll(self.scroll.get(), lines.len(), inner.height as usize);
        self.scroll.set(scroll);
        ui::render_lines(f, inner, &lines, scroll, &[]);
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::ClosePopup)
            || key_match(k, KeyAction::Quit)
            || k.code == KeyCode::Char('?')
        {
            self.ctx.queue.push(InternalEvent::ClosePopup);
            return Ok(EventState::consumed());
        }
        let mut scroll = self.scroll.get();
        if key_match(k, KeyAction::MoveDown) {
            scroll += 1;
        } else if key_match(k, KeyAction::MoveUp) {
            scroll = scroll.saturating_sub(1);
        } else if key_match(k, KeyAction::PageDown) {
            scroll += 20;
        } else if key_match(k, KeyAction::PageUp) {
            scroll = scroll.saturating_sub(20);
        } else {
            return Ok(EventState::not_consumed());
        }
        self.scroll.set(scroll);
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
    fn long_descriptions_wrap_instead_of_clipping() {
        assert_eq!(
            wrap_description("one two three four five six", 10),
            vec!["one two", "three four", "five six"]
        );
        // a single over-long word is hard-split rather than dropped
        assert_eq!(
            wrap_description("abcdefghijkl", 5),
            vec!["abcde", "fghij", "kl"]
        );
        assert_eq!(wrap_description("", 5), vec![""]);
        // no width to wrap into: keep the text on one (clipped) line
        assert_eq!(wrap_description("abc", 0), vec!["abc"]);
    }

    #[test]
    fn draw_and_close() {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut h = HelpPopup::new(&ctx);
        let t = ts::render(80, 30, |f| {
            h.draw(f, Rect::new(0, 0, 80, 30)).unwrap();
        });
        let s = ts::dump(&t);
        assert!(s.contains("svnui"), "{s}");
        assert!(s.contains("Quit svnui"), "{s}");
        assert!(s.contains("Help"), "{s}");
        // bindings are grouped by context
        assert!(s.contains("Global"), "{s}");
        assert!(s.contains("Status tab"), "{s}");
        // scroll down
        h.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();
        assert_eq!(h.scroll.get(), 1);
        // q closes
        h.event(&ts::key(crossterm::event::KeyCode::Char('q')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        // '?' also closes
        h.event(&ts::key(crossterm::event::KeyCode::Char('?')))
            .unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }
}
