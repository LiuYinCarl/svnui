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

/// Adjust `scroll` so `selection` stays inside the visible window of
/// `view_h` rows, scrolling minimally; the result is clamped against the
/// content length `len` via [`clamp_scroll`].
pub fn scroll_follow(selection: usize, scroll: usize, len: usize, view_h: usize) -> usize {
    let mut scroll = scroll;
    if view_h > 0 {
        if selection < scroll {
            scroll = selection;
        } else if selection >= scroll + view_h {
            scroll = selection - view_h + 1;
        }
    }
    clamp_scroll(scroll, len, view_h)
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

/// Render lines like [`render_lines`], but shifted right by `h_off`
/// display columns: the first `h_off` columns of every line are skipped.
/// Used by scrollable code views (diff / blame) so long lines remain
/// reachable on narrow terminals.
pub fn render_lines_h(
    f: &mut Frame,
    area: Rect,
    lines: &[Line],
    scroll: usize,
    highlights: &[(usize, Style)],
    h_off: usize,
) {
    if h_off == 0 {
        render_lines(f, area, lines, scroll, highlights);
        return;
    }
    let sliced: Vec<Line> = lines.iter().map(|l| slice_line_left(l, h_off)).collect();
    render_lines(f, area, &sliced, scroll, highlights);
}

/// Drop the first `skip` display columns of a line, keeping span styles.
/// A wide char straddling the cut boundary is replaced by a single space.
fn slice_line_left(line: &Line, skip: usize) -> Line<'static> {
    use unicode_width::UnicodeWidthChar;
    let mut col = 0usize;
    let mut spans: Vec<ratatui::text::Span> = Vec::new();
    for span in &line.spans {
        let mut text = String::new();
        for ch in span.content.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if col + w <= skip {
                // fully left of the cut
            } else if col < skip {
                text.push(' '); // straddling wide char
            } else {
                text.push(ch);
            }
            col += w;
        }
        if !text.is_empty() {
            spans.push(ratatui::text::Span::styled(text, span.style));
        }
    }
    Line::from(spans)
}

/// Clamp a horizontal scroll offset so the view cannot scroll past the
/// right edge of the widest line.
pub fn clamp_hscroll(hscroll: usize, max_width: usize, width: usize) -> usize {
    hscroll.min(max_width.saturating_sub(width))
}

/// Split off a one-row footer (e.g. the `/pattern` search line) from the
/// bottom of `inner` when `active`. Returns the (shrunk) content area and
/// the footer rect.
pub fn split_search_footer(inner: Rect, active: bool) -> (Rect, Option<Rect>) {
    if active && inner.height > 1 {
        (
            Rect::new(inner.x, inner.y, inner.width, inner.height - 1),
            Some(Rect::new(
                inner.x,
                inner.y + inner.height - 1,
                inner.width,
                1,
            )),
        )
    } else {
        (inner, None)
    }
}

/// Compute a centered popup rect with the given width/height constraints.
pub fn popup_area(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    // u32 intermediates: `width * percent` overflows u16 on wide terminals
    let width = (u32::from(area.width) * u32::from(percent_x) / 100) as u16;
    let height = (u32::from(area.height) * u32::from(percent_y) / 100) as u16;
    let width = width.max(20).min(area.width);
    let height = height.max(5).min(area.height);
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
    fn scroll_follow_keeps_selection_visible() {
        // selection above the window pulls the scroll up
        assert_eq!(scroll_follow(2, 5, 50, 10), 2);
        // selection below the window pulls the scroll down
        assert_eq!(scroll_follow(15, 5, 50, 10), 6);
        // selection inside the window: scroll unchanged
        assert_eq!(scroll_follow(7, 5, 50, 10), 5);
        // clamped against the content length
        assert_eq!(scroll_follow(49, 45, 50, 10), 40);
        // zero-height view: no follow, just the clamp
        assert_eq!(scroll_follow(3, 7, 50, 0), 0);
        // content shorter than the view: scroll resets to 0
        assert_eq!(scroll_follow(0, 3, 5, 10), 0);
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
        // very wide terminals: u16 multiplication must not overflow
        let wide = popup_area(Rect::new(0, 0, 1000, 300), 92, 92);
        assert_eq!(wide.width, 920);
        assert_eq!(wide.height, 276);
        assert!(wide.x + wide.width <= 1000);
        assert!(wide.y + wide.height <= 300);
        // popup larger than the area is clamped to it
        let clamped = popup_area(Rect::new(0, 0, 10, 4), 50, 50);
        assert!(clamped.width <= 10);
        assert!(clamped.height <= 4);
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
    fn clamp_hscroll_bounds_at_widest_line() {
        assert_eq!(clamp_hscroll(100, 50, 20), 30);
        assert_eq!(clamp_hscroll(10, 50, 20), 10);
        // content narrower than the view: no scrolling at all
        assert_eq!(clamp_hscroll(10, 10, 20), 0);
    }

    #[test]
    fn split_search_footer_carves_bottom_row() {
        let inner = Rect::new(1, 2, 40, 10);
        let (content, footer) = split_search_footer(inner, true);
        assert_eq!(content, Rect::new(1, 2, 40, 9));
        assert_eq!(footer, Some(Rect::new(1, 11, 40, 1)));
        // inactive or too small: the area is returned untouched
        assert_eq!(split_search_footer(inner, false), (inner, None));
        let tiny = Rect::new(1, 2, 40, 1);
        assert_eq!(split_search_footer(tiny, true), (tiny, None));
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

    #[test]
    fn render_lines_h_skips_columns_and_keeps_styles() {
        let backend = TestBackend::new(10, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        let red = Style::default().fg(ratatui::style::Color::Red);
        let lines = vec![
            Line::from(vec![Span::styled("abcdef", red)]),
            // "中文ab" is 2+2+1+1 = 6 columns; skipping 3 cuts the 文 in
            // half, which must collapse to a single space
            Line::from(vec![Span::styled("中文ab", red)]),
        ];
        terminal
            .draw(|f| {
                render_lines_h(f, Rect::new(0, 0, 10, 2), &lines, 0, &[], 3);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        let row: String = (0..6).map(|x| buf[(x, 0)].symbol().to_string()).collect();
        assert_eq!(row, "def   ");
        assert_eq!(buf[(0, 0)].fg, ratatui::style::Color::Red);
        let row2: String = (0..4).map(|x| buf[(x, 1)].symbol().to_string()).collect();
        assert_eq!(row2, " ab ");
        // offset 0 behaves exactly like render_lines
        terminal
            .draw(|f| {
                render_lines_h(f, Rect::new(0, 0, 10, 2), &lines, 0, &[], 0);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 0)].symbol(), "a");
        assert_eq!(buf[(0, 1)].symbol(), "中");
    }
}
