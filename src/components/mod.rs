//! UI components, modeled on gitui's `components` module.

pub mod blame;
pub mod commit;
pub mod diff_view;
pub mod file_finder;
pub mod file_log;
pub mod help;
pub mod log;
pub mod status_tree;

use crate::queue::Queue;
use crate::ui::style::Theme;
use crossterm::event::Event;
use ratatui::Frame;
use ratatui::layout::Rect;

/// Result of a component handling an input event.
#[derive(Clone, Copy, Debug, Default)]
pub struct EventState {
    /// The event was consumed by this component.
    pub consumed: bool,
}

impl EventState {
    pub const fn consumed() -> Self {
        Self { consumed: true }
    }
    pub const fn not_consumed() -> Self {
        Self { consumed: false }
    }
}

/// Shared context handed to every component.
#[derive(Clone)]
pub struct Context {
    pub queue: Queue,
    pub theme: Theme,
}

/// Anything that can be drawn and receives input events.
pub trait DrawableComponent {
    fn draw(&self, f: &mut Frame, area: Rect) -> Result<(), String>;
    fn event(&mut self, ev: &Event) -> Result<EventState, String>;
}
