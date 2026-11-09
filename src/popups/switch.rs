//! Branch / target picker for `svn switch`.
//!
//! The popup opens prefilled with the branch the working copy is on. The
//! repository's switch targets (`^/trunk`, `^/branches/*`, `^/tags/*`)
//! arrive asynchronously from `svn list`; until then the input is usable
//! on its own, so a repo with an unusual layout (or an offline server)
//! never blocks the operation.
//!
//! Typing edits the target directly — any URL or repo-relative `^/path`
//! is accepted — while ↑/↓ pick from the list. Enter hands the target to
//! the regular confirmation popup (switching a working copy is a
//! mutation, and may raise conflicts), Esc cancels.

use super::{Context, DrawableComponent, EventState};
use crate::queue::{ConfirmAction, InternalEvent};
use crate::ui::{self, style::Theme};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use std::cell::Cell;

/// What the working copy is switched to, prefixed with caret notation so
/// the list and the input show the same form (`^/branches/feat-a`).
fn caret_target(branch: &str) -> String {
    if branch.is_empty() {
        String::new()
    } else {
        format!("^/{}", branch.trim_start_matches("^/"))
    }
}

/// Normalize what the user typed into an svn switch target.
///
/// `^/path` and absolute URLs (`scheme://...`) are taken as-is; anything
/// else is treated as repo-relative (`trunk` → `^/trunk`). An empty input
/// or the bare repository root (`^/`) is not a usable target: switching a
/// trunk checkout to `^/` is rejected by svn anyway, and it is far more
/// likely a half-typed target than an intent.
fn normalize_target(input: &str) -> Option<String> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    let target = if t.starts_with("^/") || t.contains("://") {
        t.to_string()
    } else {
        format!("^/{}", t.trim_start_matches('/'))
    };
    (target != "^/" && !target.ends_with("://")).then_some(target)
}

pub struct SwitchPopup {
    ctx: Context,
    /// target being edited (`^/path` or an absolute URL)
    input: String,
    /// branch the working copy is on, marked in the list
    current: String,
    targets: Vec<String>,
    /// `svn list` request still in flight
    pub pending: bool,
    /// why the target list is empty; the input keeps working
    pub error: Option<String>,
    /// index into `targets` when picked with ↑/↓
    selection: Option<usize>,
    scroll: Cell<usize>,
}

impl SwitchPopup {
    pub fn new(ctx: &Context, current_branch: &str) -> Self {
        let current = caret_target(current_branch);
        Self {
            ctx: ctx.clone(),
            input: current.clone(),
            current,
            targets: Vec::new(),
            pending: true,
            error: None,
            selection: None,
            scroll: Cell::new(0),
        }
    }

    /// Deliver the `svn list` result. A failure is shown inline: the
    /// popup stays usable for a hand-typed target.
    pub fn update(&mut self, result: Result<Vec<String>, String>) {
        self.pending = false;
        match result {
            Ok(targets) => {
                self.targets = targets;
                self.error = None;
                // start on the current branch so the first ↑/↓ moves away
                // from it; skip if the user already typed a target while
                // the list was loading
                if self.input == self.current {
                    self.selection = self.targets.iter().position(|t| *t == self.current);
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// The target Enter would use, or None when the input is unusable.
    pub fn target(&self) -> Option<String> {
        normalize_target(&self.input)
    }

    /// Number of switch targets discovered in the repository (0 while
    /// loading or when the repo has no standard layout).
    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    /// Move the list selection and mirror it into the input, so the row
    /// under the cursor is exactly what Enter switches to.
    fn pick(&mut self, delta: isize) {
        if self.targets.is_empty() {
            return;
        }
        let idx = match self.selection {
            None => 0,
            Some(i) => ui::clamp_index((i as isize + delta).max(0) as usize, self.targets.len()),
        };
        self.selection = Some(idx);
        self.input = self.targets[idx].clone();
    }

    fn edit(&mut self, f: impl FnOnce(&mut String)) {
        f(&mut self.input);
        // typed text wins over the list cursor: the highlight would
        // otherwise claim a row the input no longer matches
        self.selection = None;
    }

    fn list_line(&self, idx: usize, theme: &Theme) -> Line<'static> {
        let target = self.targets[idx].clone();
        let style = if target == self.current {
            theme.dim
        } else {
            theme.text
        };
        let mut spans = vec![Span::styled(target, style)];
        if self.targets[idx] == self.current {
            spans.push(Span::styled("  (current)", theme.dim));
        }
        Line::from(spans)
    }
}

impl DrawableComponent for SwitchPopup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        let theme = &self.ctx.theme;
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.popup_border))
            .title(crate::strings::TITLE.switch);
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

        ui::render_line_at(
            f,
            chunks[0].x,
            chunks[0].y,
            chunks[0].width,
            &Line::from(vec![
                Span::styled("Target: ", theme.info),
                Span::raw(self.input.clone()),
            ]),
        );

        ui::render_line_at(
            f,
            chunks[2].x,
            chunks[2].y,
            chunks[2].width,
            &Line::from(Span::styled(
                "Enter: switch  ↑/↓: pick  Esc: cancel",
                theme.dim,
            )),
        );

        if self.pending {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    crate::strings::MSG.loading,
                    theme.dim,
                ))),
                chunks[1],
            );
            return Ok(());
        }
        if self.targets.is_empty() {
            let msg = match &self.error {
                Some(e) => format!("Cannot list branches: {e}"),
                None => "No branches found — type a target (^/path or URL)".to_string(),
            };
            let style = if self.error.is_some() {
                theme.error
            } else {
                theme.dim
            };
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(msg, style))),
                chunks[1],
            );
            return Ok(());
        }

        let view_h = chunks[1].height as usize;
        let scroll = ui::scroll_follow(
            self.selection.unwrap_or(0),
            self.scroll.get(),
            self.targets.len(),
            view_h,
        );
        self.scroll.set(scroll);

        let end = (scroll + view_h).min(self.targets.len());
        let lines: Vec<Line> = (scroll..end).map(|i| self.list_line(i, theme)).collect();
        let mut highlights = Vec::new();
        if let Some(sel) = self.selection
            && sel >= scroll
            && sel < end
        {
            highlights.push((
                sel - scroll,
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
            self.edit(|input| input.push_str(text));
            return Ok(EventState::consumed());
        }
        let Event::Key(k) = ev else {
            return Ok(EventState::not_consumed());
        };
        match k.code {
            KeyCode::Esc => {
                self.ctx.queue.push(InternalEvent::ClosePopup);
            }
            KeyCode::Enter => {
                if let Some(target) = self.target() {
                    // close first: the app opens the confirmation popup
                    // from the queue, right after this ClosePopup
                    self.ctx.queue.push(InternalEvent::ClosePopup);
                    self.ctx
                        .queue
                        .push(InternalEvent::Confirm(ConfirmAction::Switch(target)));
                }
            }
            KeyCode::Up => self.pick(-1),
            KeyCode::Down => self.pick(1),
            KeyCode::Backspace => self.edit(|input| {
                input.pop();
            }),
            KeyCode::Char(c)
                if !k
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.edit(|input| input.push(c));
            }
            _ => return Ok(EventState::not_consumed()),
        }
        Ok(EventState::consumed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::Queue;
    use crate::test_support as ts;

    fn popup(current: &str) -> (SwitchPopup, Queue) {
        let q = Queue::new();
        let ctx = Context {
            queue: q.clone(),
            theme: Theme::default(),
        };
        (SwitchPopup::new(&ctx, current), q)
    }

    fn down(p: &mut SwitchPopup) {
        p.event(&ts::key(KeyCode::Down)).unwrap();
    }

    fn up(p: &mut SwitchPopup) {
        p.event(&ts::key(KeyCode::Up)).unwrap();
    }

    fn type_str(p: &mut SwitchPopup, s: &str) {
        for c in s.chars() {
            p.event(&ts::key(KeyCode::Char(c))).unwrap();
        }
    }

    fn ctrl(code: KeyCode) -> Event {
        Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::CONTROL))
    }

    #[test]
    fn opens_prefilled_with_the_current_branch() {
        let (p, _q) = popup("branches/feat-a");
        assert_eq!(p.input, "^/branches/feat-a");
        assert_eq!(p.current, "^/branches/feat-a");
        assert!(p.pending);
        assert_eq!(p.target().as_deref(), Some("^/branches/feat-a"));
        // an empty branch (svn reported no Relative URL) leaves the input blank
        let (p2, _q2) = popup("");
        assert!(p2.input.is_empty());
        assert_eq!(p2.target(), None);
    }

    #[test]
    fn targets_arrive_async_and_errors_stay_inline() {
        let (mut p, _q) = popup("trunk");
        p.update(Ok(vec!["^/trunk".into(), "^/branches/feat-a".into()]));
        assert!(!p.pending);
        assert_eq!(p.targets.len(), 2);
        assert!(p.error.is_none());

        let (mut p2, _q2) = popup("trunk");
        p2.update(Err("network down".into()));
        assert!(!p2.pending);
        assert_eq!(p2.error.as_deref(), Some("network down"));
        assert!(p2.targets.is_empty());
        // the input still works with no list
        type_str(&mut p2, "x");
        assert_eq!(p2.target().as_deref(), Some("^/trunkx"));
    }

    #[test]
    fn arrows_pick_from_the_list_and_mirror_into_the_input() {
        let (mut p, _q) = popup("trunk");
        p.update(Ok(vec![
            "^/trunk".into(),
            "^/branches/a".into(),
            "^/branches/b".into(),
        ]));
        // the cursor starts on the current branch
        assert_eq!(p.selection, Some(0));
        assert_eq!(p.input, "^/trunk");
        down(&mut p);
        assert_eq!(p.input, "^/branches/a");
        down(&mut p);
        down(&mut p); // clamped at the last entry
        assert_eq!(p.input, "^/branches/b");
        up(&mut p);
        assert_eq!(p.input, "^/branches/a");
        up(&mut p);
        up(&mut p); // clamped at the first entry
        assert_eq!(p.input, "^/trunk");
        assert_eq!(p.selection, Some(0));
    }

    #[test]
    fn typed_input_survives_a_late_target_list() {
        let (mut p, _q) = popup("trunk");
        for _ in 0..p.input.len() {
            p.event(&ts::key(KeyCode::Backspace)).unwrap();
        }
        type_str(&mut p, "branches/z");
        p.update(Ok(vec!["^/trunk".into(), "^/branches/z".into()]));
        assert_eq!(p.selection, None, "typing must win over the list cursor");
        assert_eq!(p.target().as_deref(), Some("^/branches/z"));
    }

    #[test]
    fn arrows_without_a_list_are_harmless() {
        let (mut p, _q) = popup("trunk");
        down(&mut p);
        up(&mut p);
        assert_eq!(p.selection, None);
        assert_eq!(p.input, "^/trunk");
    }

    #[test]
    fn typing_overrides_the_list_selection() {
        let (mut p, _q) = popup("");
        p.update(Ok(vec!["^/trunk".into(), "^/branches/a".into()]));
        down(&mut p);
        assert_eq!(p.selection, Some(0));
        // the prefilled input is replaced, not appended to
        for _ in 0..p.input.len() {
            p.event(&ts::key(KeyCode::Backspace)).unwrap();
        }
        type_str(&mut p, "branches/z");
        assert_eq!(p.selection, None);
        assert_eq!(p.target().as_deref(), Some("^/branches/z"));
    }

    #[test]
    fn enter_confirms_the_normalized_target_and_closes() {
        let (mut p, q) = popup("");
        type_str(&mut p, "branches/feat-a");
        p.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Confirm(ConfirmAction::Switch(t)))
                if t == "^/branches/feat-a"
        ));
    }

    #[test]
    fn enter_keeps_absolute_urls_and_leading_caret() {
        let (mut p, q) = popup("");
        type_str(&mut p, "https://svn.example.com/repos/p/branches/x");
        p.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(matches!(
            q.pop(),
            Some(InternalEvent::Confirm(ConfirmAction::Switch(t)))
                if t == "https://svn.example.com/repos/p/branches/x"
        ));

        let (mut p2, q2) = popup("");
        type_str(&mut p2, "^/trunk");
        p2.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(matches!(q2.pop(), Some(InternalEvent::ClosePopup)));
        assert!(matches!(
            q2.pop(),
            Some(InternalEvent::Confirm(ConfirmAction::Switch(t))) if t == "^/trunk"
        ));
    }

    #[test]
    fn enter_with_an_empty_or_root_target_does_nothing() {
        let (mut p, q) = popup("");
        p.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(q.pop().is_none(), "empty target must not confirm");

        type_str(&mut p, "^/");
        p.event(&ts::key(KeyCode::Enter)).unwrap();
        assert!(
            q.pop().is_none(),
            "the bare repository root is not a target"
        );
    }

    #[test]
    fn esc_closes_without_confirming() {
        let (mut p, q) = popup("trunk");
        let state = p.event(&ts::key(KeyCode::Esc)).unwrap();
        assert!(state.consumed);
        assert!(matches!(q.pop(), Some(InternalEvent::ClosePopup)));
        assert!(q.pop().is_none());
    }

    #[test]
    fn paste_edits_the_input() {
        let (mut p, _q) = popup("");
        let ev = Event::Paste("^/tags/v1.0".to_string());
        assert!(p.event(&ev).unwrap().consumed);
        assert_eq!(p.target().as_deref(), Some("^/tags/v1.0"));
    }

    #[test]
    fn ctrl_keys_are_left_to_the_app() {
        let (mut p, _q) = popup("trunk");
        let ev = ctrl(KeyCode::Char('s'));
        assert!(!p.event(&ev).unwrap().consumed);
    }

    #[test]
    fn draw_shows_input_targets_and_the_current_marker() {
        let (mut p, _q) = popup("trunk");
        p.update(Ok(vec!["^/trunk".into(), "^/branches/feat-a".into()]));
        let t = ts::render(80, 12, |f| {
            p.draw(f, Rect::new(0, 0, 80, 12)).unwrap();
        });
        let dump = ts::dump(&t);
        assert!(dump.contains("Switch working copy"), "{dump}");
        assert!(dump.contains("Target: ^/trunk"), "{dump}");
        assert!(dump.contains("(current)"), "{dump}");
        assert!(dump.contains("^/branches/feat-a"), "{dump}");
    }

    #[test]
    fn draw_pending_and_error_states() {
        let (p, _q) = popup("trunk");
        let t = ts::render(80, 12, |f| {
            p.draw(f, Rect::new(0, 0, 80, 12)).unwrap();
        });
        assert!(ts::dump(&t).contains("Loading..."));

        let (mut p2, _q2) = popup("trunk");
        p2.update(Err("offline".into()));
        let t2 = ts::render(80, 12, |f| {
            p2.draw(f, Rect::new(0, 0, 80, 12)).unwrap();
        });
        let dump = ts::dump(&t2);
        assert!(dump.contains("Cannot list branches: offline"), "{dump}");
        assert!(dump.contains("Target: ^/trunk"), "{dump}");

        let (mut p3, _q3) = popup("trunk");
        p3.update(Ok(vec![]));
        let t3 = ts::render(80, 12, |f| {
            p3.draw(f, Rect::new(0, 0, 80, 12)).unwrap();
        });
        assert!(ts::dump(&t3).contains("No branches found"));
    }
}
