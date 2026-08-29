//! Fuzzy file finder popup (fzf-style): type a keyword, pick a versioned
//! file, Enter opens its history, Ctrl+b opens blame.
//!
//! The file list comes from `svn list -R` (loaded async: the popup shows
//! "Loading..." until `update` delivers it). Matching is delegated to the
//! `fuzzy-matcher` crate (skim's matcher): subsequence matching with proper
//! scoring (consecutive/word-boundary bonuses, smart case) plus the matched
//! character positions, which we highlight in the result list. A bare
//! letter is always query text, so blame needs the Ctrl modifier here.

use super::{Context, DrawableComponent, EventState};
use crate::keys::{KeyAction, key_match};
use crate::ui::{self, style::Theme};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear};
use std::cell::Cell;

/// Cap the rendered result list; filtering itself is not capped.
const MAX_RESULTS: usize = 200;

pub struct FileFinderPopup {
    ctx: Context,
    pub files: Vec<String>,
    pub pending: bool,
    query: String,
    /// Boxed: the matcher carries sizeable internal buffers, which would
    /// otherwise bloat the `Popup` enum (clippy::large_enum_variant)
    matcher: Box<SkimMatcherV2>,
    /// `(index into files, matched char indices)`, best match first
    filtered: Vec<(usize, Vec<usize>)>,
    selection: usize,
    scroll: Cell<usize>,
}

impl FileFinderPopup {
    pub fn new(ctx: &Context) -> Self {
        Self {
            ctx: ctx.clone(),
            files: Vec::new(),
            pending: true,
            query: String::new(),
            matcher: Box::new(SkimMatcherV2::default()),
            filtered: Vec::new(),
            selection: 0,
            scroll: Cell::new(0),
        }
    }

    pub fn update(&mut self, files: Vec<String>) {
        self.pending = false;
        self.files = files;
        self.refilter();
    }

    fn refilter(&mut self) {
        let mut scored: Vec<(i64, usize, Vec<usize>)> = self
            .files
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                self.matcher
                    .fuzzy_indices(f, &self.query)
                    .map(|(score, idx)| (score, i, idx))
            })
            .collect();
        // best score first; ties keep the original path order
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        self.filtered = scored
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, i, idx)| (i, idx))
            .collect();
        self.selection = 0;
        self.scroll.set(0);
    }

    fn selected_path(&self) -> Option<String> {
        let (idx, _) = self.filtered.get(self.selection)?;
        self.files.get(*idx).cloned()
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.filtered.len();
        if len == 0 {
            return;
        }
        self.selection = ui::clamp_index((self.selection as isize + delta).max(0) as usize, len);
    }
}

impl DrawableComponent for FileFinderPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(format!(
                "{} ({} files)",
                crate::strings::TITLE.file_finder,
                self.files.len()
            ));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(inner);

        // input row
        ui::render_line_at(
            f,
            chunks[0].x,
            chunks[0].y,
            chunks[0].width,
            &Line::from(vec![
                Span::styled("> ", theme.info),
                Span::raw(self.query.clone()),
            ]),
        );

        // footer: key hints (blame needs Ctrl here — bare 'b' is query text)
        ui::render_line_at(
            f,
            chunks[2].x,
            chunks[2].y,
            chunks[2].width,
            &Line::from(Span::styled(
                "Enter history  ^B blame  Esc close",
                theme.dim,
            )),
        );

        if self.pending {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                    crate::strings::MSG.loading,
                    theme.dim,
                ))),
                chunks[1],
            );
            return Ok(());
        }
        if self.filtered.is_empty() {
            f.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                    "No matching files",
                    theme.dim,
                ))),
                chunks[1],
            );
            return Ok(());
        }

        let view_h = chunks[1].height as usize;
        let scroll = ui::scroll_follow(
            self.selection,
            self.scroll.get(),
            self.filtered.len(),
            view_h,
        );
        self.scroll.set(scroll);

        let end = (scroll + view_h).min(self.filtered.len());
        let mut lines: Vec<Line> = Vec::with_capacity(end - scroll);
        for (i, matched) in &self.filtered[scroll..end] {
            lines.push(result_line(&self.files[*i], matched, theme));
        }
        let mut highlights = Vec::new();
        if self.selection >= scroll && self.selection < end {
            highlights.push((
                self.selection - scroll,
                Style::default()
                    .bg(theme.selection_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        ui::render_lines(f, chunks[1], &lines, 0, &highlights);
        Ok(())
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        if let Event::Paste(text) = ev {
            self.query.push_str(text);
            self.refilter();
            return Ok(EventState::consumed());
        }
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        if key_match(k, KeyAction::BlameFileFinder) {
            if let Some(path) = self.selected_path() {
                self.ctx
                    .queue
                    .push(crate::queue::InternalEvent::RequestBlame(path));
            }
            return Ok(EventState::consumed());
        }
        match k.code {
            KeyCode::Esc => {
                self.ctx.queue.push(crate::queue::InternalEvent::ClosePopup);
            }
            KeyCode::Enter => {
                if let Some(path) = self.selected_path() {
                    self.ctx.queue.push(crate::queue::InternalEvent::ClosePopup);
                    self.ctx
                        .queue
                        .push(crate::queue::InternalEvent::OpenFileHistory(path));
                }
            }
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down => self.move_selection(1),
            KeyCode::PageUp => self.move_selection(-20),
            KeyCode::PageDown => self.move_selection(20),
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
            }
            KeyCode::Char(c)
                if !k
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(c);
                self.refilter();
            }
            _ => return Ok(EventState::not_consumed()),
        }
        Ok(EventState::consumed())
    }
}

/// A result row: the path with the fuzzy-matched characters highlighted.
fn result_line(path: &str, matched: &[usize], theme: &Theme) -> Line<'static> {
    let match_style = Style::default()
        .fg(theme.info.fg.unwrap_or(ratatui::style::Color::Cyan))
        .add_modifier(Modifier::BOLD);
    let spans: Vec<Span> = path
        .chars()
        .enumerate()
        .map(|(i, c)| {
            if matched.contains(&i) {
                Span::styled(c.to_string(), match_style)
            } else {
                Span::styled(c.to_string(), theme.text)
            }
        })
        .collect();
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::InternalEvent;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn comp() -> (FileFinderPopup, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut p = FileFinderPopup::new(&ctx);
        p.update(vec![
            "src/main.rs".into(),
            "src/components/status_tree.rs".into(),
            "docs/readme.md".into(),
            "Cargo.toml".into(),
        ]);
        (p, q)
    }

    fn type_str(p: &mut FileFinderPopup, s: &str) {
        for ch in s.chars() {
            p.event(&ts::key(KeyCode::Char(ch))).unwrap();
        }
    }

    #[test]
    fn fuzzy_matching_smart_case_and_ordering() {
        // subsequence match across path segments
        let (mut p, _q) = comp();
        type_str(&mut p, "smr");
        assert_eq!(p.selected_path().as_deref(), Some("src/main.rs"));
        // smart case: lowercase query matches uppercase letters
        let (mut p, _q) = comp();
        type_str(&mut p, "cargo");
        assert_eq!(p.selected_path().as_deref(), Some("Cargo.toml"));
        // non-subsequence yields nothing
        let (mut p, _q) = comp();
        type_str(&mut p, "xyz");
        assert!(p.selected_path().is_none());
        // consecutive matches rank above scattered ones
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut p = FileFinderPopup::new(&ctx);
        p.update(vec!["m_a_i_n.rs".into(), "main.rs".into()]);
        type_str(&mut p, "main");
        assert_eq!(p.selected_path().as_deref(), Some("main.rs"));
    }

    #[test]
    fn matched_chars_are_highlighted() {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut p = FileFinderPopup::new(&ctx);
        p.update(vec!["xab.rs".into()]);
        type_str(&mut p, "ab");
        let t = ts::render(60, 10, |f| {
            p.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        let buf = t.backend().buffer();
        // first result row is y=2 (border + input row); "xab.rs" at x=1:
        // matched chars 'a','b' are drawn in the info color, others not
        // (the row selection bolds the whole row, so check fg instead)
        assert_eq!(buf[(1, 2)].fg, ratatui::style::Color::Gray);
        assert_eq!(buf[(2, 2)].fg, ratatui::style::Color::Cyan);
        assert_eq!(buf[(3, 2)].fg, ratatui::style::Color::Cyan);
        assert_eq!(buf[(4, 2)].fg, ratatui::style::Color::Gray);
    }

    #[test]
    fn typing_filters_and_enter_opens_history() {
        let (mut p, q) = comp();
        type_str(&mut p, "stree");
        assert_eq!(
            p.selected_path().as_deref(),
            Some("src/components/status_tree.rs")
        );
        p.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::OpenFileHistory(path)) if path == "src/components/status_tree.rs"
        ));
    }

    #[test]
    fn ctrl_b_blames_highlighted_file() {
        let (mut p, q) = comp();
        type_str(&mut p, "stree");
        assert_eq!(
            p.selected_path().as_deref(),
            Some("src/components/status_tree.rs")
        );
        // Ctrl+b requests blame and keeps the finder open (no ClosePopup)
        let ctrl_b = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('b'),
            KeyModifiers::CONTROL,
        ));
        assert!(p.event(&ctrl_b).unwrap().consumed);
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::RequestBlame(path)) if path == "src/components/status_tree.rs"
        ));
        assert!(q.pop().is_none());
        // a bare 'b' is query text, not blame
        type_str(&mut p, "b");
        assert!(q.pop().is_none());
        // no selection (empty result list): no event
        let (mut p2, q2) = comp();
        type_str(&mut p2, "zzz-no-match");
        p2.event(&ctrl_b).unwrap();
        assert!(q2.pop().is_none());
    }

    #[test]
    fn navigation_backspace_esc() {
        let (mut p, q) = comp();
        assert_eq!(p.selected_path().as_deref(), Some("src/main.rs"));
        p.event(&ts::key(KeyCode::Down)).unwrap();
        assert_eq!(
            p.selected_path().as_deref(),
            Some("src/components/status_tree.rs")
        );
        p.event(&ts::key(KeyCode::Up)).unwrap();
        p.event(&ts::key(KeyCode::PageDown)).unwrap();
        p.event(&ts::key(KeyCode::PageUp)).unwrap();
        assert_eq!(p.selected_path().as_deref(), Some("src/main.rs"));
        // backspace edits the query
        type_str(&mut p, "zz");
        assert!(p.selected_path().is_none());
        p.event(&ts::key(KeyCode::Backspace)).unwrap();
        p.event(&ts::key(KeyCode::Backspace)).unwrap();
        assert_eq!(p.selected_path().as_deref(), Some("src/main.rs"));
        // ctrl combos are not query text
        let ctrl_c = Event::Key(crossterm::event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        ));
        p.event(&ctrl_c).unwrap();
        assert!(p.query.is_empty());
        // paste appends to the query
        p.event(&Event::Paste("readme".into())).unwrap();
        assert_eq!(p.selected_path().as_deref(), Some("docs/readme.md"));
        // Esc closes
        p.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
    }

    #[test]
    fn draw_states() {
        let q = crate::queue::Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        let mut p = FileFinderPopup::new(&ctx);
        let t1 = ts::render(60, 10, |f| {
            p.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t1).contains("Loading"));
        p.update(vec!["a.txt".into()]);
        let t2 = ts::render(60, 10, |f| {
            p.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        let s = ts::dump(&t2);
        assert!(s.contains("Find file"), "{s}");
        assert!(s.contains("a.txt"), "{s}");
        assert!(s.contains("^B blame"), "{s}");
        // no match state
        type_str(&mut p, "qqq");
        let t3 = ts::render(60, 10, |f| {
            p.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t3).contains("No matching files"));
    }
}
