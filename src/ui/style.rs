//! Color theme (hardcoded, gitui-style).

use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Debug)]
pub struct Theme {
    pub selection_bg: Color,
    pub staged_bg: Color,
    pub border_focused: Color,
    pub border_unfocused: Color,
    pub title_focused: Style,
    pub text: Style,
    pub dim: Style,

    // status codes
    pub status_modified: Style,
    pub status_added: Style,
    pub status_deleted: Style,
    pub status_conflicted: Style,
    pub status_unversioned: Style,
    pub status_ignored: Style,
    pub status_missing: Style,
    pub status_other: Style,

    // diff
    pub diff_header: Style,
    pub diff_file_header: Style,
    pub diff_hunk: Style,
    pub diff_added: Style,
    pub diff_removed: Style,
    pub diff_note: Style,
    pub diff_line_number: Style,

    // log
    pub log_revision: Style,
    pub log_author: Style,
    pub log_message: Style,
    pub log_action_added: Style,
    pub log_action_deleted: Style,
    pub log_action_modified: Style,
    pub log_action_other: Style,

    // blame
    pub blame_author: Style,
    pub blame_rev_alt: [Style; 12],

    // incremental search (diff / blame popups)
    pub search_hit: Style,
    pub search_hit_current: Style,

    // popups
    pub popup_border: Color,
    pub confirm_yes: Style,
    pub confirm_no: Style,

    pub error: Style,
    pub info: Style,
}

impl Default for Theme {
    fn default() -> Self {
        let selection = Color::Rgb(0x3b, 0x42, 0x61);
        let staged = Color::Rgb(0x2a, 0x4a, 0x3a);
        Self {
            selection_bg: selection,
            staged_bg: staged,
            border_focused: Color::Cyan,
            border_unfocused: Color::DarkGray,
            title_focused: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            text: Style::default().fg(Color::Gray),
            dim: Style::default().fg(Color::DarkGray),
            status_modified: Style::default().fg(Color::Yellow),
            status_added: Style::default().fg(Color::Green),
            status_deleted: Style::default().fg(Color::Red),
            status_conflicted: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            status_unversioned: Style::default().fg(Color::Cyan),
            status_ignored: Style::default().fg(Color::DarkGray),
            status_missing: Style::default().fg(Color::Red),
            status_other: Style::default().fg(Color::Magenta),
            diff_header: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
            diff_file_header: Style::default().fg(Color::DarkGray),
            diff_hunk: Style::default().fg(Color::Cyan),
            diff_added: Style::default().fg(Color::Green),
            diff_removed: Style::default().fg(Color::Red),
            diff_note: Style::default().fg(Color::DarkGray),
            diff_line_number: Style::default().fg(Color::DarkGray),
            log_revision: Style::default().fg(Color::Yellow),
            log_author: Style::default().fg(Color::Cyan),
            log_message: Style::default().fg(Color::Gray),
            log_action_added: Style::default().fg(Color::Green),
            log_action_deleted: Style::default().fg(Color::Red),
            log_action_modified: Style::default().fg(Color::Yellow),
            log_action_other: Style::default().fg(Color::Magenta),
            blame_author: Style::default().fg(Color::Cyan),
            search_hit: Style::default().fg(Color::Black).bg(Color::Yellow),
            search_hit_current: Style::default()
                .fg(Color::Black)
                .bg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
            blame_rev_alt: [
                Style::default().fg(Color::Red),
                Style::default().fg(Color::Green),
                Style::default().fg(Color::Yellow),
                Style::default().fg(Color::Blue),
                Style::default().fg(Color::Magenta),
                Style::default().fg(Color::Cyan),
                Style::default().fg(Color::LightRed),
                Style::default().fg(Color::LightGreen),
                Style::default().fg(Color::LightYellow),
                Style::default().fg(Color::LightBlue),
                Style::default().fg(Color::LightMagenta),
                Style::default().fg(Color::LightCyan),
            ],
            popup_border: Color::Yellow,
            confirm_yes: Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            confirm_no: Style::default().fg(Color::Red),
            error: Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            info: Style::default().fg(Color::Cyan),
        }
    }
}

impl Theme {
    /// Status style for an svn status code char.
    pub fn status_style(&self, code: char) -> Style {
        match code {
            'M' => self.status_modified,
            'A' => self.status_added,
            'D' => self.status_deleted,
            'C' => self.status_conflicted,
            '?' => self.status_unversioned,
            'I' => self.status_ignored,
            '!' => self.status_missing,
            'R' | '~' | 'X' => self.status_other,
            _ => self.text,
        }
    }

    /// Log changed-path action style.
    pub fn log_action_style(&self, action: char) -> Style {
        match action {
            'A' => self.log_action_added,
            'D' => self.log_action_deleted,
            'M' | 'R' => self.log_action_modified,
            _ => self.log_action_other,
        }
    }
}
