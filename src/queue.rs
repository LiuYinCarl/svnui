//! Single-threaded event queue for components to communicate (like gitui's
//! `queue` module).

use bitflags::bitflags;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

bitflags! {
    /// Which parts of the app need a refresh/redraw.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct NeedsUpdate: u32 {
        /// redraw everything
        const ALL = 0b0111;
        /// status tree / diff may have changed
        const STATUS = 0b0010;
        /// log view may have changed
        const LOG = 0b0100;
    }
}

/// Requests that components push to the app for processing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InternalEvent {
    /// Refresh request with flags
    Update(NeedsUpdate),
    /// Show an info message popup
    ShowInfoMsg(String),
    /// Open the help popup
    OpenHelp,
    /// Close the top popup
    ClosePopup,
    /// Focus the commit input
    OpenCommit,
    /// Ask for confirmation of a pending action
    Confirm(ConfirmAction),
    /// The user confirmed an action (pushed by the confirm popup)
    Confirmed(ConfirmAction),
    /// Switch to a tab
    SwitchTab(Tab),
    /// Refresh status from the working copy
    RefreshStatus,
    /// Run svn add on the given paths
    AddFiles(Vec<String>),
    /// Request a diff of the currently selected file (fullscreen)
    RequestFileDiff,
    /// Request blame of the selected file
    RequestBlame,
    /// Request a diff of the selected revision (log tab)
    RequestRevisionDiff(u64),
}

/// An action that needs user confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmAction {
    /// Commit with message and optional paths
    Commit { message: String, paths: Vec<String> },
    /// Revert the given paths
    Revert(Vec<String>),
    /// Update the working copy
    Update,
    /// Resolve conflict on a path
    Resolve(String),
    /// Update working copy to a revision
    UpdateToRevision(u64),
}

/// Which main tab is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Status,
    Log,
}

/// Simple FIFO queue shared via Rc, so components can push events without
/// holding a reference to the app.
#[derive(Clone, Default)]
pub struct Queue {
    data: Rc<RefCell<VecDeque<InternalEvent>>>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, ev: InternalEvent) {
        self.data.borrow_mut().push_back(ev);
    }

    pub fn pop(&self) -> Option<InternalEvent> {
        self.data.borrow_mut().pop_front()
    }

    pub fn drain(&self) -> Vec<InternalEvent> {
        let mut v = Vec::new();
        while let Some(ev) = self.pop() {
            v.push(ev);
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_order() {
        let q = Queue::new();
        q.push(InternalEvent::OpenHelp);
        q.push(InternalEvent::RefreshStatus);
        assert_eq!(q.pop(), Some(InternalEvent::OpenHelp));
        assert_eq!(q.pop(), Some(InternalEvent::RefreshStatus));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn drain_returns_all_and_clears() {
        let q = Queue::new();
        q.push(InternalEvent::ClosePopup);
        q.push(InternalEvent::ClosePopup);
        let drained = q.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn needs_update_flags() {
        let all = NeedsUpdate::ALL;
        assert!(all.contains(NeedsUpdate::STATUS));
        assert!(all.contains(NeedsUpdate::LOG));
        let s = NeedsUpdate::STATUS;
        assert!(!s.contains(NeedsUpdate::LOG));
        assert_eq!(s | NeedsUpdate::LOG, NeedsUpdate::STATUS | NeedsUpdate::LOG);
    }
}
