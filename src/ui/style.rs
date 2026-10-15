//! Color theme (hardcoded, gitui-style).
//!
//! 统一配色规则：颜色只承载以下四种语义，组件不得硬编码颜色；新增颜色
//! 一律先在这里定义为 `palette` 常量或 `Theme` 字段。
//!
//! - 绿 = 安全/正常：新增、diff 增加行、提交集、成功、确认
//! - 橙 = 警告：修改、未版本化、落后远端、需要注意的变更
//! - 红 = 危险/错误：删除、冲突、缺失、失败、确认否
//! - 灰 = 不重要：已忽略、其他状态、日期、行号、提示
//!
//! 不承载状态语义的只有结构色：`palette::ACCENT`（焦点边框/标题、
//! 修订号/作者等标识）、选中/暂存的背景色，以及 blame 按修订轮换的
//! `blame_rev_alt` 身份色板。

use ratatui::style::{Color, Modifier, Style};

/// 统一语义色板：全项目唯一的状态颜色来源。
///
/// `TEXT` 是普通正文色，`ACCENT` 是结构强调色（焦点、标识），两者都
/// 不承载状态语义。
pub mod palette {
    use ratatui::style::Color;

    /// 绿：安全/正常（新增、成功、提交集）。
    pub const SAFE: Color = Color::Green;
    /// 橙：警告/需要注意（修改、未版本化、落后）。
    ///
    /// 用 xterm-256 的橙色：`Color::Yellow` 是暗黄，无法表达"警告"。
    pub const WARN: Color = Color::Indexed(208);
    /// 红：危险/错误（删除、冲突、缺失、失败）。
    pub const DANGER: Color = Color::Red;
    /// 灰：不重要的辅助信息（忽略项、日期、行号、提示）。
    pub const MUTED: Color = Color::DarkGray;
    /// 普通正文。
    pub const TEXT: Color = Color::Gray;
    /// 结构强调：焦点边框/标题、修订号/作者等标识。
    pub const ACCENT: Color = Color::Cyan;
    /// 搜索命中的背景色（非当前命中；当前命中直接用 `WARN`）。
    pub const HIT_BG: Color = Color::Indexed(94);
}

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
            border_focused: palette::ACCENT,
            border_unfocused: palette::MUTED,
            title_focused: Style::default()
                .fg(palette::ACCENT)
                .add_modifier(Modifier::BOLD),
            text: Style::default().fg(palette::TEXT),
            dim: Style::default().fg(palette::MUTED),

            // status codes：绿安全 / 橙警告 / 红危险 / 灰不重要
            status_modified: Style::default().fg(palette::WARN),
            status_added: Style::default().fg(palette::SAFE),
            status_deleted: Style::default().fg(palette::DANGER),
            status_conflicted: Style::default()
                .fg(palette::DANGER)
                .add_modifier(Modifier::BOLD),
            status_unversioned: Style::default().fg(palette::WARN),
            status_ignored: Style::default().fg(palette::MUTED),
            status_missing: Style::default().fg(palette::DANGER),
            status_other: Style::default().fg(palette::MUTED),

            // diff
            diff_header: Style::default()
                .fg(palette::ACCENT)
                .add_modifier(Modifier::BOLD),
            diff_file_header: Style::default().fg(palette::MUTED),
            diff_hunk: Style::default().fg(palette::ACCENT),
            diff_added: Style::default().fg(palette::SAFE),
            diff_removed: Style::default().fg(palette::DANGER),
            diff_note: Style::default().fg(palette::MUTED),
            diff_line_number: Style::default().fg(palette::MUTED),

            // log
            log_revision: Style::default().fg(palette::ACCENT),
            log_author: Style::default().fg(palette::ACCENT),
            log_message: Style::default().fg(palette::TEXT),
            log_action_added: Style::default().fg(palette::SAFE),
            log_action_deleted: Style::default().fg(palette::DANGER),
            log_action_modified: Style::default().fg(palette::WARN),
            log_action_other: Style::default().fg(palette::MUTED),

            // blame
            blame_author: Style::default().fg(palette::ACCENT),
            // 身份色板：仅按修订号轮换以区分相邻修订块，不承载状态语义
            blame_rev_alt: [
                Style::default().fg(palette::DANGER),
                Style::default().fg(palette::SAFE),
                Style::default().fg(palette::WARN),
                Style::default().fg(Color::Blue),
                Style::default().fg(Color::Magenta),
                Style::default().fg(palette::ACCENT),
                Style::default().fg(Color::LightRed),
                Style::default().fg(Color::LightGreen),
                Style::default().fg(Color::Indexed(215)), // 浅橙
                Style::default().fg(Color::LightBlue),
                Style::default().fg(Color::LightMagenta),
                Style::default().fg(Color::LightCyan),
            ],

            // incremental search (diff / blame popups)：橙色系背景高亮，
            // 亮底/暗底区分"当前命中"与"其他命中"
            search_hit: Style::default().fg(Color::White).bg(palette::HIT_BG),
            search_hit_current: Style::default()
                .fg(Color::Black)
                .bg(palette::WARN)
                .add_modifier(Modifier::BOLD),

            // popups
            popup_border: palette::ACCENT,
            confirm_yes: Style::default()
                .fg(palette::SAFE)
                .add_modifier(Modifier::BOLD),
            confirm_no: Style::default().fg(palette::DANGER),

            error: Style::default()
                .fg(palette::DANGER)
                .add_modifier(Modifier::BOLD),
            info: Style::default().fg(palette::ACCENT),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 语义色板是本文件的契约：绿=安全、橙=警告、红=危险、灰=不重要。
    /// 任何字段改色都要在这里同步体现，避免再出现"黄/品红各表一意"。
    #[test]
    fn semantic_palette_is_uniform() {
        let t = Theme::default();

        // 安全 -> 绿
        for style in [
            t.status_added,
            t.diff_added,
            t.log_action_added,
            t.confirm_yes,
        ] {
            assert_eq!(style.fg, Some(palette::SAFE));
        }

        // 警告 -> 橙
        for style in [
            t.status_modified,
            t.status_unversioned,
            t.log_action_modified,
        ] {
            assert_eq!(style.fg, Some(palette::WARN));
        }
        assert_eq!(t.search_hit_current.bg, Some(palette::WARN));

        // 危险/错误 -> 红
        for style in [
            t.status_deleted,
            t.status_conflicted,
            t.status_missing,
            t.diff_removed,
            t.log_action_deleted,
            t.confirm_no,
            t.error,
        ] {
            assert_eq!(style.fg, Some(palette::DANGER));
        }

        // 不重要 -> 灰
        for style in [
            t.status_ignored,
            t.status_other,
            t.log_action_other,
            t.dim,
            t.diff_file_header,
            t.diff_note,
            t.diff_line_number,
        ] {
            assert_eq!(style.fg, Some(palette::MUTED));
        }

        // 结构色不承载状态语义：焦点/标识用强调色
        assert_eq!(t.border_focused, palette::ACCENT);
        assert_eq!(t.popup_border, palette::ACCENT);
        assert_eq!(t.log_revision.fg, Some(palette::ACCENT));
        assert_eq!(t.log_author.fg, Some(palette::ACCENT));
        assert_eq!(t.info.fg, Some(palette::ACCENT));
        assert_eq!(t.text.fg, Some(palette::TEXT));
    }

    #[test]
    fn status_and_action_codes_map_to_the_right_semantics() {
        let t = Theme::default();
        assert_eq!(t.status_style('M').fg, Some(palette::WARN));
        assert_eq!(t.status_style('A').fg, Some(palette::SAFE));
        assert_eq!(t.status_style('D').fg, Some(palette::DANGER));
        assert_eq!(t.status_style('C').fg, Some(palette::DANGER));
        assert_eq!(t.status_style('!').fg, Some(palette::DANGER));
        assert_eq!(t.status_style('?').fg, Some(palette::WARN));
        assert_eq!(t.status_style('I').fg, Some(palette::MUTED));
        for other in ['R', '~', 'X'] {
            assert_eq!(t.status_style(other).fg, Some(palette::MUTED));
        }
        assert_eq!(t.status_style('z').fg, Some(palette::TEXT));

        assert_eq!(t.log_action_style('A').fg, Some(palette::SAFE));
        assert_eq!(t.log_action_style('D').fg, Some(palette::DANGER));
        for modified in ['M', 'R'] {
            assert_eq!(t.log_action_style(modified).fg, Some(palette::WARN));
        }
        assert_eq!(t.log_action_style('X').fg, Some(palette::MUTED));
    }

    /// 语义色板里不再有黄色/品红前景：黄=旧的"警示"色，已被橙色取代。
    #[test]
    fn legacy_yellow_and_magenta_are_gone() {
        let t = Theme::default();
        let fields = [
            t.status_modified,
            t.status_added,
            t.status_deleted,
            t.status_conflicted,
            t.status_unversioned,
            t.status_ignored,
            t.status_missing,
            t.status_other,
            t.diff_header,
            t.diff_file_header,
            t.diff_hunk,
            t.diff_added,
            t.diff_removed,
            t.diff_note,
            t.diff_line_number,
            t.log_revision,
            t.log_author,
            t.log_message,
            t.log_action_added,
            t.log_action_deleted,
            t.log_action_modified,
            t.log_action_other,
            t.blame_author,
            t.search_hit,
            t.search_hit_current,
            t.confirm_yes,
            t.confirm_no,
            t.error,
            t.info,
            t.text,
            t.dim,
        ];
        for style in fields {
            assert_ne!(style.fg, Some(Color::Yellow));
            assert_ne!(style.fg, Some(Color::LightYellow));
            assert_ne!(style.fg, Some(Color::Magenta));
            assert_ne!(style.fg, Some(Color::LightMagenta));
            assert_ne!(style.bg, Some(Color::Yellow));
            assert_ne!(style.bg, Some(Color::Magenta));
        }
    }
}
