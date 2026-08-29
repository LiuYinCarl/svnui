//! Popups: temporary modal dialogs drawn on top of the active tab.
//!
//! The popup stack is a `Vec<Popup>`; the enum avoids downcasting.

pub mod confirm;
pub mod diff;
pub mod msg;
pub mod output;

use super::components::{Context, DrawableComponent, EventState};
use crate::ui;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;

pub use confirm::ConfirmPopup;
pub use diff::DiffPopup;
pub use msg::MsgPopup;
pub use output::OutputPopup;

/// One entry in the popup stack.
pub enum Popup {
    Confirm(ConfirmPopup),
    Msg(MsgPopup),
    Help(super::components::help::HelpPopup),
    Output(OutputPopup),
    Diff(DiffPopup),
    Blame(super::components::blame::BlamePopup),
    FileLog(super::components::file_log::FileLogPopup),
    FileFinder(super::components::file_finder::FileFinderPopup),
    LogSearch(super::components::log_search::LogSearchPopup),
    StatusFilter(super::components::status_filter::StatusFilterPopup),
}

impl Popup {
    /// The rectangle this popup occupies within the screen.
    pub fn rect(&self, area: Rect) -> Rect {
        match self {
            Popup::Confirm(_) => ui::popup_area(area, 60, 40),
            Popup::Msg(_) => ui::popup_area(area, 65, 35),
            Popup::Help(_) => ui::popup_area(area, 75, 75),
            Popup::Output(_) => ui::popup_area(area, 80, 65),
            Popup::Diff(_) => ui::popup_area(area, 92, 92),
            Popup::Blame(_) => ui::popup_area(area, 92, 92),
            Popup::FileLog(_) => ui::popup_area(area, 75, 70),
            Popup::FileFinder(_) => ui::popup_area(area, 70, 60),
            Popup::LogSearch(_) => ui::popup_area(area, 60, 20),
            Popup::StatusFilter(_) => ui::popup_area(area, 60, 20),
        }
    }
}

impl DrawableComponent for Popup {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String> {
        match self {
            Popup::Confirm(p) => p.draw(f, area),
            Popup::Msg(p) => p.draw(f, area),
            Popup::Help(p) => p.draw(f, area),
            Popup::Output(p) => p.draw(f, area),
            Popup::Diff(p) => p.draw(f, area),
            Popup::Blame(p) => p.draw(f, area),
            Popup::FileLog(p) => p.draw(f, area),
            Popup::FileFinder(p) => p.draw(f, area),
            Popup::LogSearch(p) => p.draw(f, area),
            Popup::StatusFilter(p) => p.draw(f, area),
        }
    }

    fn event(&mut self, ev: &Event) -> Result<EventState, String> {
        match self {
            Popup::Confirm(p) => p.event(ev),
            Popup::Msg(p) => p.event(ev),
            Popup::Help(p) => p.event(ev),
            Popup::Output(p) => p.event(ev),
            Popup::Diff(p) => p.event(ev),
            Popup::Blame(p) => p.event(ev),
            Popup::FileLog(p) => p.event(ev),
            Popup::FileFinder(p) => p.event(ev),
            Popup::LogSearch(p) => p.event(ev),
            Popup::StatusFilter(p) => p.event(ev),
        }
    }
}

// Convenience constructors used by the app
impl Popup {
    pub fn confirm(ctx: &Context, message: String, action: crate::queue::ConfirmAction) -> Self {
        Popup::Confirm(ConfirmPopup::new(ctx, message, action))
    }
    pub fn msg(ctx: &Context, message: String, is_error: bool) -> Self {
        Popup::Msg(MsgPopup::new(ctx, message, is_error))
    }
    pub fn help(ctx: &Context) -> Self {
        Popup::Help(super::components::help::HelpPopup::new(ctx))
    }
    pub fn output(ctx: &Context, title: String, content: &str) -> Self {
        Popup::Output(OutputPopup::new(ctx, title, content))
    }
    pub fn blame(ctx: &Context, path: &str) -> Self {
        Popup::Blame(super::components::blame::BlamePopup::new(ctx, path))
    }
    pub fn file_log(ctx: &Context, path: &str) -> Self {
        Popup::FileLog(super::components::file_log::FileLogPopup::new(ctx, path))
    }
    pub fn file_finder(ctx: &Context) -> Self {
        Popup::FileFinder(super::components::file_finder::FileFinderPopup::new(ctx))
    }
    pub fn log_search(ctx: &Context, initial: &str) -> Self {
        Popup::LogSearch(super::components::log_search::LogSearchPopup::new(
            ctx, initial,
        ))
    }
    pub fn status_filter(ctx: &Context, initial: &str) -> Self {
        Popup::StatusFilter(super::components::status_filter::StatusFilterPopup::new(
            ctx, initial,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::ConfirmAction;
    use crate::test_support as ts;
    use crate::ui::style::Theme;

    fn ctx() -> (Context, crate::queue::Queue) {
        let q = crate::queue::Queue::new();
        (
            Context {
                queue: q.clone(),
                theme: Theme::default(),
            },
            q,
        )
    }

    #[test]
    fn rects_are_centered_and_valid() {
        let (c, _q) = ctx();
        let area = Rect::new(0, 0, 120, 40);
        let popups = vec![
            Popup::confirm(&c, "m".into(), ConfirmAction::Update),
            Popup::msg(&c, "m".into(), false),
            Popup::help(&c),
            Popup::output(&c, "t".into(), "c"),
            Popup::Diff(DiffPopup::new(&c, "t".into(), "c")),
            Popup::blame(&c, "p"),
            Popup::file_log(&c, "p"),
            Popup::file_finder(&c),
            Popup::log_search(&c, ""),
            Popup::status_filter(&c, ""),
        ];
        for p in &popups {
            let r = p.rect(area);
            assert!(r.width > 0 && r.height > 0, "{r:?}");
            assert!(r.x + r.width <= 120);
            assert!(r.y + r.height <= 40);
        }
    }

    #[test]
    fn draw_and_event_dispatch_through_enum() {
        let (c, q) = ctx();
        let mut p = Popup::msg(&c, "hello popup".into(), false);
        let t = ts::render(60, 10, |f| {
            p.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t).contains("hello popup"));
        // event dispatch: any key closes the msg popup
        p.event(&ts::key(crossterm::event::KeyCode::Char('x')))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(crate::queue::InternalEvent::ClosePopup)
        ));

        let mut c2 = Popup::confirm(&c, "sure?".into(), ConfirmAction::Update);
        let t2 = ts::render(60, 10, |f| {
            c2.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t2).contains("sure?"));
        c2.event(&ts::key(crossterm::event::KeyCode::Char('y')))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(crate::queue::InternalEvent::ClosePopup)
        ));
        assert!(matches!(
            q.pop(),
            Some(crate::queue::InternalEvent::Confirmed(_))
        ));

        let h = Popup::help(&c);
        let t3 = ts::render(60, 20, |f| {
            h.draw(f, Rect::new(0, 0, 60, 20)).unwrap();
        });
        assert!(ts::dump(&t3).contains("Quit svnui"));

        let mut o = Popup::output(&c, "out".into(), "some output");
        let t4 = ts::render(60, 10, |f| {
            o.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t4).contains("some output"));
        o.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();

        let d = Popup::Diff(DiffPopup::new(
            &c,
            "d".into(),
            "Index: x\n===\n@@ -1 +1 @@\n-a\n+b\n",
        ));
        let t5 = ts::render(60, 10, |f| {
            d.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t5).contains("Index: x"));

        let mut b = Popup::blame(&c, "f.rs");
        let t6 = ts::render(60, 10, |f| {
            b.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t6).contains("Blame: f.rs"));
        b.event(&ts::key(crossterm::event::KeyCode::Char('j')))
            .unwrap();

        let mut fl = Popup::file_log(&c, "f.rs");
        let t7 = ts::render(60, 10, |f| {
            fl.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t7).contains("File history: f.rs"));
        fl.event(&ts::key(crossterm::event::KeyCode::Char('q')))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(crate::queue::InternalEvent::ClosePopup)
        ));

        let mut ff = Popup::file_finder(&c);
        let t8 = ts::render(60, 10, |f| {
            ff.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t8).contains("Find file"));
        ff.event(&ts::key(crossterm::event::KeyCode::Esc)).unwrap();
        assert!(matches!(
            q.pop(),
            Some(crate::queue::InternalEvent::ClosePopup)
        ));

        // earlier popups in this test intentionally leave events in the
        // queue (e.g. confirm's ClosePopup); start clean
        while q.pop().is_some() {}

        let mut ls = Popup::log_search(&c, "ini");
        let t9 = ts::render(60, 10, |f| {
            ls.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t9).contains("Search commits"));
        ls.event(&ts::key(crossterm::event::KeyCode::Enter))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(crate::queue::InternalEvent::SearchLog(s)) if s == "ini"
        ));
        assert!(matches!(
            q.pop(),
            Some(crate::queue::InternalEvent::ClosePopup)
        ));

        let mut sf = Popup::status_filter(&c, "ini");
        let t10 = ts::render(60, 10, |f| {
            sf.draw(f, Rect::new(0, 0, 60, 10)).unwrap();
        });
        assert!(ts::dump(&t10).contains("Filter status files"));
        sf.event(&ts::key(crossterm::event::KeyCode::Enter))
            .unwrap();
        assert!(matches!(
            q.pop(),
            Some(crate::queue::InternalEvent::ClosePopup)
        ));
    }
}
