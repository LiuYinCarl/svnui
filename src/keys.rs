//! Key bindings and key matching helpers.
//!
//! Modeled loosely on gitui's `keys` module: a central place that maps
//! crossterm key events to logical actions. Keybindings are hardcoded
//! here (no config file support yet).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A description of a keybinding, used for the help popup and status bar.
#[derive(Clone, Debug)]
pub struct KeyBinding {
    /// Human readable key string, e.g. "j / ↓"
    pub keys: String,
    /// What the key does
    pub description: String,
}

impl KeyBinding {
    pub fn new(keys: &'static str, description: &'static str) -> Self {
        Self {
            keys: keys.to_string(),
            description: description.to_string(),
        }
    }
}

/// Logical actions mapped from keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    PageUp,
    PageDown,
    Home,
    End,
    Enter,
    Escape,
    Quit,
    ToggleStage,
    StageAll,
    UnstageAll,
    AddFiles,
    RevertFiles,
    ResolveConflict,
    Commit,
    UpdateWc,
    DiffFull,
    Blame,
    Filter,
    Refresh,
    SwitchTabStatus,
    SwitchTabLog,
    Help,
    Confirm,
    Deny,
    ClosePopup,
    FocusNext,
    FocusPrev,
    CommitConfirm,
    OpenRevisionDiff,
    UpdateToRevision,
    FileHistory,
    OpenFileFinder,
    ToggleMark,
    ViewCommitInfo,
    BlameFileFinder,
    SwitchTabPatches,
    /// Save the working-copy changes as a patch file (`P`, app-level)
    SavePatch,
    /// Patches tab: preview the selected patch (`p`, Enter is separate)
    PreviewPatch,
    /// Patches tab: apply the selected patch (`a`)
    ApplyPatch,
    /// Patches tab: delete the selected patch file (`d`)
    DeletePatch,
    /// Log tab / file history popup: scroll the detail pane down (Ctrl+d)
    DetailScrollDown,
    /// Log tab / file history popup: scroll the detail pane up (Ctrl+u)
    DetailScrollUp,
}

/// Central keybindings. `KeyAction::None` is used for unbound keys.
pub fn key_match(ev: &KeyEvent, action: KeyAction) -> bool {
    match action {
        KeyAction::MoveUp => is_key(ev, KeyCode::Up) || is_key(ev, KeyCode::Char('k')),
        KeyAction::MoveDown => is_key(ev, KeyCode::Down) || is_key(ev, KeyCode::Char('j')),
        KeyAction::MoveLeft => is_key(ev, KeyCode::Left) || is_key(ev, KeyCode::Char('h')),
        KeyAction::MoveRight => is_key(ev, KeyCode::Right) || is_key(ev, KeyCode::Char('l')),
        KeyAction::PageUp => is_key(ev, KeyCode::PageUp) || is_key(ev, KeyCode::Char('K')),
        KeyAction::PageDown => is_key(ev, KeyCode::PageDown) || is_key(ev, KeyCode::Char('J')),
        KeyAction::Home => is_key(ev, KeyCode::Home) || is_key(ev, KeyCode::Char('g')),
        KeyAction::End => is_key(ev, KeyCode::End) || is_key(ev, KeyCode::Char('G')),
        KeyAction::Enter => is_key(ev, KeyCode::Enter),
        KeyAction::Escape => is_key(ev, KeyCode::Esc),
        KeyAction::Quit => is_key(ev, KeyCode::Char('q')),
        KeyAction::ToggleStage => is_key(ev, KeyCode::Char(' ')),
        KeyAction::StageAll => is_key(ev, KeyCode::Char('A')),
        KeyAction::UnstageAll => is_key(ev, KeyCode::Char('U')),
        KeyAction::AddFiles => is_key(ev, KeyCode::Char('a')),
        KeyAction::RevertFiles => is_key(ev, KeyCode::Char('r')),
        KeyAction::ResolveConflict => is_key(ev, KeyCode::Char('x')),
        KeyAction::Commit => is_key(ev, KeyCode::Char('c')),
        KeyAction::UpdateWc => is_key(ev, KeyCode::Char('u')),
        KeyAction::DiffFull => is_key(ev, KeyCode::Char('d')),
        KeyAction::Blame => is_key(ev, KeyCode::Char('b')),
        KeyAction::Filter => is_key(ev, KeyCode::Char('/')),
        KeyAction::Refresh => is_key(ev, KeyCode::F(5)) || is_key(ev, KeyCode::Char('R')),
        KeyAction::SwitchTabStatus => is_key(ev, KeyCode::Char('1')),
        KeyAction::SwitchTabLog => is_key(ev, KeyCode::Char('2')),
        KeyAction::SwitchTabPatches => is_key(ev, KeyCode::Char('3')),
        KeyAction::Help => is_key(ev, KeyCode::Char('?')),
        KeyAction::Confirm => is_key(ev, KeyCode::Char('y')) || is_key(ev, KeyCode::Char('Y')),
        KeyAction::Deny => is_key(ev, KeyCode::Char('n')) || is_key(ev, KeyCode::Char('N')),
        KeyAction::ClosePopup => is_key(ev, KeyCode::Esc),
        KeyAction::FocusNext => {
            is_key(ev, KeyCode::Tab) && !ev.modifiers.contains(KeyModifiers::SHIFT)
        }
        KeyAction::FocusPrev => {
            is_key(ev, KeyCode::BackTab)
                || (is_key(ev, KeyCode::Tab) && ev.modifiers.contains(KeyModifiers::SHIFT))
        }
        KeyAction::CommitConfirm => {
            (ev.code == KeyCode::Enter
                && !ev
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT))
                || (ev.code == KeyCode::Char('s') && ev.modifiers.contains(KeyModifiers::CONTROL))
        }
        KeyAction::OpenRevisionDiff => is_key(ev, KeyCode::Enter) || is_key(ev, KeyCode::Char('d')),
        KeyAction::UpdateToRevision => is_key(ev, KeyCode::Char('o')),
        KeyAction::FileHistory => is_key(ev, KeyCode::Char('t')),
        KeyAction::OpenFileFinder => {
            ev.code == KeyCode::Char('p') && ev.modifiers.contains(KeyModifiers::CONTROL)
        }
        KeyAction::ToggleMark => is_key(ev, KeyCode::Char(' ')),
        KeyAction::ViewCommitInfo => is_key(ev, KeyCode::Char('v')),
        // In the file finder a bare 'b' is query text, so blame uses Ctrl+b
        KeyAction::BlameFileFinder => {
            ev.code == KeyCode::Char('b') && ev.modifiers.contains(KeyModifiers::CONTROL)
        }
        KeyAction::SavePatch => is_key(ev, KeyCode::Char('P')),
        // plain 'p' is free: the file finder uses Ctrl+p
        KeyAction::PreviewPatch => is_key(ev, KeyCode::Char('p')),
        // 'a'/'d' are also AddFiles/DiffFull; the patch actions only exist
        // in the patches tab, which consumes the key before the status tab
        // would ever see it
        KeyAction::ApplyPatch => is_key(ev, KeyCode::Char('a')),
        KeyAction::DeletePatch => is_key(ev, KeyCode::Char('d')),
        // Ctrl+d/u scroll the detail pane; plain d/u are separate actions
        // and are matched without modifiers via `is_key`
        KeyAction::DetailScrollDown => {
            ev.code == KeyCode::Char('d') && ev.modifiers.contains(KeyModifiers::CONTROL)
        }
        KeyAction::DetailScrollUp => {
            ev.code == KeyCode::Char('u') && ev.modifiers.contains(KeyModifiers::CONTROL)
        }
    }
}

/// Match a single key code ignoring modifiers like SHIFT for letters,
/// but rejecting CONTROL/ALT combos (Alt+q must not quit, etc.).
fn is_key(ev: &KeyEvent, code: KeyCode) -> bool {
    ev.code == code
        && !ev
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

/// A titled section of the help popup: the bindings of one context
/// (global keys, one per tab, popup-specific keys).
pub struct KeyGroup {
    pub title: &'static str,
    pub bindings: Vec<KeyBinding>,
}

/// Build the default binding list for the help popup, grouped by the
/// context in which the keys are active.
pub fn all_binding_groups() -> Vec<KeyGroup> {
    vec![
        KeyGroup {
            title: "Global",
            bindings: vec![
                KeyBinding::new("1 / 2 / 3", "Switch tab: status / log / patches"),
                KeyBinding::new(
                    "Tab / Shift+Tab",
                    "Status tab: cycle pane focus; log/patches: switch tab",
                ),
                KeyBinding::new("Ctrl+p", "Fuzzy find a versioned file"),
                KeyBinding::new("P", "Save working-copy changes as a patch file (no revert)"),
                KeyBinding::new("F5 / R", "Refresh status / log / patch list"),
                KeyBinding::new("?", "Show this help"),
                KeyBinding::new("q", "Quit svnui"),
            ],
        },
        KeyGroup {
            title: "Status tab",
            bindings: vec![
                KeyBinding::new("j / ↓ / k / ↑", "Move selection"),
                KeyBinding::new("h / ← / l / →", "Collapse / expand directory"),
                KeyBinding::new("g / G", "Jump to first / last entry"),
                KeyBinding::new("PgUp / PgDn", "Page up / down"),
                KeyBinding::new("space", "Stage / unstage (toggle commit set)"),
                KeyBinding::new("A / U", "Stage all changes / unstage all"),
                KeyBinding::new("a", "Add selected files (svn add)"),
                KeyBinding::new("r", "Revert selected files (svn revert)"),
                KeyBinding::new("x", "Resolve conflict (accept working copy)"),
                KeyBinding::new("Enter / d", "Diff of the selected file"),
                KeyBinding::new("b", "Blame file (svn blame)"),
                KeyBinding::new("t", "File history (svn log of selected file)"),
                KeyBinding::new("/", "Filter files"),
                KeyBinding::new("c", "Focus commit message / commit"),
                KeyBinding::new("Enter / Ctrl+s", "Commit (in commit input)"),
                KeyBinding::new("Tab", "Commit input: pick a recent message"),
                KeyBinding::new("u", "Update working copy (svn update)"),
            ],
        },
        KeyGroup {
            title: "Log tab",
            bindings: vec![
                KeyBinding::new("Enter / d", "Diff of selected / marked revisions"),
                KeyBinding::new("space", "Mark / unmark revision (range diff)"),
                KeyBinding::new("v", "Show full commit info"),
                KeyBinding::new("o", "Update working copy to selected revision"),
                KeyBinding::new("/", "Filter loaded commits / search all history"),
                KeyBinding::new("Ctrl+d / Ctrl+u", "Scroll commit details down / up"),
            ],
        },
        KeyGroup {
            title: "Patches tab",
            bindings: vec![
                KeyBinding::new("Enter / p", "Preview patch (diff view)"),
                KeyBinding::new("a", "Apply patch (svn patch, confirmed)"),
                KeyBinding::new("d", "Delete patch file (confirmed)"),
            ],
        },
        KeyGroup {
            title: "Popups",
            bindings: vec![
                KeyBinding::new("/", "Diff / blame popup: search text (live)"),
                KeyBinding::new("n / N", "Diff / blame search: next / previous match"),
                KeyBinding::new("Ctrl+b", "File finder: blame highlighted file"),
                KeyBinding::new("b", "File history popup: blame the file"),
                KeyBinding::new(
                    "Ctrl+d / Ctrl+u",
                    "File history popup: scroll commit message",
                ),
                KeyBinding::new("Esc", "Close popup / cancel / clear search highlights"),
            ],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn navigation_keys() {
        assert!(key_match(&key(KeyCode::Char('j')), KeyAction::MoveDown));
        assert!(key_match(&key(KeyCode::Down), KeyAction::MoveDown));
        assert!(!key_match(&key(KeyCode::Char('j')), KeyAction::MoveUp));
        assert!(key_match(&key(KeyCode::Char('k')), KeyAction::MoveUp));
        assert!(key_match(&key(KeyCode::Up), KeyAction::MoveUp));
        assert!(key_match(&key(KeyCode::Char('h')), KeyAction::MoveLeft));
        assert!(key_match(&key(KeyCode::Left), KeyAction::MoveLeft));
        assert!(key_match(&key(KeyCode::Char('l')), KeyAction::MoveRight));
        assert!(key_match(&key(KeyCode::Right), KeyAction::MoveRight));
        assert!(key_match(&key(KeyCode::PageUp), KeyAction::PageUp));
        assert!(key_match(&key(KeyCode::Char('K')), KeyAction::PageUp));
        assert!(key_match(&key(KeyCode::PageDown), KeyAction::PageDown));
        assert!(key_match(&key(KeyCode::Char('J')), KeyAction::PageDown));
        assert!(key_match(&key(KeyCode::Home), KeyAction::Home));
        assert!(key_match(&key(KeyCode::Char('g')), KeyAction::Home));
        assert!(key_match(&key(KeyCode::End), KeyAction::End));
        assert!(key_match(&key(KeyCode::Char('G')), KeyAction::End));
    }

    #[test]
    fn action_keys() {
        assert!(key_match(&key(KeyCode::Enter), KeyAction::Enter));
        assert!(key_match(&key(KeyCode::Esc), KeyAction::Escape));
        assert!(key_match(&key(KeyCode::Char('q')), KeyAction::Quit));
        assert!(!key_match(&key(KeyCode::Char('q')), KeyAction::Commit));
        assert!(key_match(&key(KeyCode::Char(' ')), KeyAction::ToggleStage));
        assert!(key_match(&key(KeyCode::Char('A')), KeyAction::StageAll));
        assert!(key_match(&key(KeyCode::Char('U')), KeyAction::UnstageAll));
        // lowercase a/u keep their own actions, no overlap with A/U
        assert!(!key_match(&key(KeyCode::Char('a')), KeyAction::StageAll));
        assert!(!key_match(&key(KeyCode::Char('u')), KeyAction::UnstageAll));
        assert!(key_match(&key(KeyCode::Char('a')), KeyAction::AddFiles));
        assert!(key_match(&key(KeyCode::Char('r')), KeyAction::RevertFiles));
        assert!(key_match(
            &key(KeyCode::Char('x')),
            KeyAction::ResolveConflict
        ));
        assert!(key_match(&key(KeyCode::Char('c')), KeyAction::Commit));
        assert!(key_match(&key(KeyCode::Char('u')), KeyAction::UpdateWc));
        assert!(key_match(&key(KeyCode::Char('d')), KeyAction::DiffFull));
        assert!(key_match(&key(KeyCode::Char('b')), KeyAction::Blame));
        assert!(key_match(&key(KeyCode::Char('/')), KeyAction::Filter));
        assert!(key_match(&key(KeyCode::F(5)), KeyAction::Refresh));
        assert!(key_match(&key(KeyCode::Char('R')), KeyAction::Refresh));
        assert!(key_match(
            &key(KeyCode::Char('1')),
            KeyAction::SwitchTabStatus
        ));
        assert!(key_match(&key(KeyCode::Char('2')), KeyAction::SwitchTabLog));
        assert!(key_match(&key(KeyCode::Char('?')), KeyAction::Help));
        assert!(key_match(&key(KeyCode::Char('y')), KeyAction::Confirm));
        assert!(key_match(&key(KeyCode::Char('Y')), KeyAction::Confirm));
        assert!(key_match(&key(KeyCode::Char('n')), KeyAction::Deny));
        assert!(key_match(&key(KeyCode::Esc), KeyAction::ClosePopup));
        assert!(key_match(
            &key(KeyCode::Char('o')),
            KeyAction::UpdateToRevision
        ));
        assert!(key_match(&key(KeyCode::Enter), KeyAction::OpenRevisionDiff));
        assert!(key_match(
            &key(KeyCode::Char('d')),
            KeyAction::OpenRevisionDiff
        ));
        assert!(key_match(&key(KeyCode::Char('t')), KeyAction::FileHistory));
        let ctrl_p = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(key_match(&ctrl_p, KeyAction::OpenFileFinder));
        // plain 'p' must not open the finder
        assert!(!key_match(
            &key(KeyCode::Char('p')),
            KeyAction::OpenFileFinder
        ));
        assert!(key_match(&key(KeyCode::Char(' ')), KeyAction::ToggleMark));
        assert!(key_match(
            &key(KeyCode::Char('v')),
            KeyAction::ViewCommitInfo
        ));
        // 'v' must not clash with other log-tab actions
        assert!(!key_match(&key(KeyCode::Char('v')), KeyAction::Blame));
        assert!(!key_match(&key(KeyCode::Char('v')), KeyAction::Quit));
        // finder blame is Ctrl+b; plain 'b' is query text there
        let ctrl_b = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert!(key_match(&ctrl_b, KeyAction::BlameFileFinder));
        assert!(!key_match(
            &key(KeyCode::Char('b')),
            KeyAction::BlameFileFinder
        ));
        // ... and plain 'b' stays the regular blame action elsewhere
        assert!(key_match(&key(KeyCode::Char('b')), KeyAction::Blame));
        assert!(!key_match(&ctrl_b, KeyAction::Blame));
        // patch management keys
        assert!(key_match(&key(KeyCode::Char('P')), KeyAction::SavePatch));
        assert!(!key_match(&key(KeyCode::Char('p')), KeyAction::SavePatch));
        assert!(key_match(
            &key(KeyCode::Char('3')),
            KeyAction::SwitchTabPatches
        ));
        assert!(key_match(&key(KeyCode::Char('p')), KeyAction::PreviewPatch));
        assert!(key_match(&key(KeyCode::Char('a')), KeyAction::ApplyPatch));
        assert!(key_match(&key(KeyCode::Char('d')), KeyAction::DeletePatch));
        // 'p' must not clash with the finder (Ctrl+p) or the save key
        assert!(!key_match(&ctrl_p, KeyAction::PreviewPatch));
        assert!(!key_match(&ctrl_p, KeyAction::SavePatch));
    }

    #[test]
    fn tab_and_commit_confirm() {
        assert!(key_match(&key(KeyCode::Tab), KeyAction::FocusNext));
        let shift_tab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert!(key_match(&shift_tab, KeyAction::FocusPrev));
        // Enter commits (no shift)
        assert!(key_match(&key(KeyCode::Enter), KeyAction::CommitConfirm));
        // Ctrl+s commits
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert!(key_match(&ctrl_s, KeyAction::CommitConfirm));
        // plain 's' does not
        assert!(!key_match(
            &key(KeyCode::Char('s')),
            KeyAction::CommitConfirm
        ));
        // ctrl-modified letters are rejected by plain char matches
        let ctrl_j = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL);
        assert!(!key_match(&ctrl_j, KeyAction::MoveDown));
    }

    #[test]
    fn modifier_hygiene() {
        // Alt+letter must not trigger bare-letter actions
        let alt_q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT);
        assert!(!key_match(&alt_q, KeyAction::Quit));
        let alt_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT);
        assert!(!key_match(&alt_c, KeyAction::Commit));
        // Ctrl+Enter / Alt+Enter must not commit
        let ctrl_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL);
        assert!(!key_match(&ctrl_enter, KeyAction::CommitConfirm));
        let alt_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT);
        assert!(!key_match(&alt_enter, KeyAction::CommitConfirm));
        // Shift stays tolerated for letters (caps-lock style typing)
        let shift_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SHIFT);
        assert!(key_match(&shift_a, KeyAction::AddFiles));
        assert!(key_match(&shift_a, KeyAction::ApplyPatch));
        let shift_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::SHIFT);
        assert!(key_match(&shift_d, KeyAction::DiffFull));
        // Ctrl+d/u scroll the detail pane; plain d/u do not
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);
        let ctrl_u = KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL);
        assert!(key_match(&ctrl_d, KeyAction::DetailScrollDown));
        assert!(key_match(&ctrl_u, KeyAction::DetailScrollUp));
        assert!(!key_match(
            &key(KeyCode::Char('d')),
            KeyAction::DetailScrollDown
        ));
        assert!(!key_match(
            &key(KeyCode::Char('u')),
            KeyAction::DetailScrollUp
        ));
        assert!(!key_match(&ctrl_d, KeyAction::DetailScrollUp));
        // ... and the Ctrl combos don't fire the plain-letter actions
        assert!(!key_match(&ctrl_d, KeyAction::DiffFull));
        assert!(!key_match(&ctrl_u, KeyAction::UpdateWc));
    }

    #[test]
    fn bindings_list_is_nonempty_and_sorted_ok() {
        let groups = all_binding_groups();
        assert!(groups.len() >= 5);
        for g in &groups {
            assert!(!g.title.is_empty());
            assert!(!g.bindings.is_empty());
            for b in &g.bindings {
                assert!(!b.keys.is_empty());
                assert!(!b.description.is_empty());
            }
        }
    }
}
