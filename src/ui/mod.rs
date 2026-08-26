//! Shared UI helpers: scrolling, line rendering, popup rects.

pub mod style;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;

/// Clamp a scroll offset so the view stays within the content.
pub fn clamp_scroll(scroll: usize, len: usize, height: usize) -> usize {
    if height == 0 || len <= height {
        return 0;
    }
    scroll.min(len.saturating_sub(height))
}

/// Clamp a selection index into range.
pub fn clamp_index(idx: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    idx.min(len - 1)
}

/// Render a slice of styled lines into `area` with a scroll offset.
/// `highlights` is a list of (line index, style) applied as full-width
/// backgrounds before drawing the text (used for selection & staged marks).
pub fn render_lines(
    f: &mut Frame,
    area: Rect,
    lines: &[Line],
    scroll: usize,
    highlights: &[(usize, Style)],
) {
    let scroll = clamp_scroll(scroll, lines.len(), area.height as usize);
    for (i, line) in lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(area.height as usize)
    {
        let y = area.y + (i - scroll) as u16;
        if y >= area.y + area.height {
            break;
        }
        for (idx, style) in highlights {
            if *idx == i {
                f.buffer_mut()
                    .set_style(Rect::new(area.x, y, area.width, 1), *style);
            }
        }
        f.buffer_mut().set_line(area.x, y, line, area.width);
    }
}

/// Render a single line at the given position (used for inputs).
pub fn render_line_at(f: &mut Frame, x: u16, y: u16, width: u16, line: &Line) {
    f.buffer_mut().set_line(x, y, line, width);
}

/// Compute a centered popup rect with the given width/height constraints.
pub fn popup_area(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let width = (area.width * percent_x / 100).max(20);
    let height = (area.height * percent_y / 100).max(5);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

/// A small spinner: returns the next frame for a 0..=n index.
pub fn spinner_frame(i: usize) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[i % FRAMES.len()]
}

/// Truncate a string to a max char count for popup titles etc.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::text::Span;

    #[test]
    fn clamp_helpers() {
        assert_eq!(clamp_scroll(100, 5, 10), 0);
        assert_eq!(clamp_scroll(100, 50, 10), 40);
        assert_eq!(clamp_scroll(3, 50, 10), 3);
        assert_eq!(clamp_scroll(0, 50, 0), 0);
        assert_eq!(clamp_index(100, 0), 0);
        assert_eq!(clamp_index(100, 3), 2);
        assert_eq!(clamp_index(1, 3), 1);
    }

    #[test]
    fn popup_area_centered_and_bounded() {
        let r = popup_area(Rect::new(0, 0, 100, 40), 50, 50);
        assert_eq!(r.width, 50);
        assert_eq!(r.height, 20);
        assert_eq!(r.x, 25);
        assert_eq!(r.y, 10);
        // tiny screens still get usable popups
        let small = popup_area(Rect::new(0, 0, 30, 8), 10, 10);
        assert!(small.width >= 20);
        assert!(small.height >= 5);
    }

    #[test]
    fn spinner_frames_cycle() {
        assert_eq!(spinner_frame(0), spinner_frame(10));
        assert_ne!(spinner_frame(0), spinner_frame(1));
        let mut seen = std::collections::HashSet::new();
        for i in 0..10 {
            seen.insert(spinner_frame(i));
        }
        assert_eq!(seen.len(), 10);
    }

    #[test]
    fn truncate_keeps_ellipsis() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("a very long string", 8), "a very …");
    }

    #[test]
    fn render_lines_clips_and_highlights() {
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let lines: Vec<Line> = (0..10)
            .map(|i| Line::from(Span::raw(format!("line {i}"))))
            .collect();
        terminal
            .draw(|f| {
                render_lines(
                    f,
                    Rect::new(0, 0, 20, 3),
                    &lines,
                    7,
                    &[(7, Style::default().bg(ratatui::style::Color::Red))],
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let s: String = (0..3)
            .flat_map(|y| (0..20).map(move |x| buf[(x, y)].symbol().to_string()))
            .collect();
        assert!(s.contains("line 7"));
        assert!(s.contains("line 8"));
        assert!(s.contains("line 9"));
        assert!(!s.contains("line 6"));
        // highlight applied
        assert_eq!(buf[(0, 0)].bg, ratatui::style::Color::Red);
    }

    #[test]
    fn render_line_at_draws_single_line() {
        let backend = TestBackend::new(10, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render_line_at(
                    f,
                    2,
                    0,
                    8,
                    &Line::from(Span::styled("hi", ratatui::style::Style::default())),
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(2, 0)].symbol(), "h");
        assert_eq!(buf[(3, 0)].symbol(), "i");
    }
}
