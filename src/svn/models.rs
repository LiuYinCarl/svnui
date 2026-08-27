//! Data models for SVN entities.

/// Working copy info from `svn info`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvnInfo {
    /// Full URL of the working copy root
    pub url: String,
    /// Branch path relative to the repository root (e.g. "trunk",
    /// "branches/feature-x"); empty when svn reports no Relative URL
    pub branch: String,
    /// Current working copy revision
    pub revision: u64,
    /// Local path of the working copy root ("Working Copy Root Path:")
    pub wc_root: String,
}

impl SvnInfo {
    /// Short label for the status bar / confirm dialogs.
    pub fn branch_label(&self) -> &str {
        if self.branch.is_empty() {
            &self.url
        } else {
            &self.branch
        }
    }
}

/// A single entry from `svn status`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    /// Text status (first column), e.g. 'M', 'A', 'D', '?', 'C', '!', 'I'
    pub status: char,
    /// Props status (second column)
    pub props_status: char,
    /// Tree conflict marker (seventh column), 'C' when present
    pub tree_conflict: char,
    /// Path relative to the working copy root (as reported by svn)
    pub path: String,
    /// Whether the path is a directory on disk
    pub is_dir: bool,
}

impl StatusEntry {
    pub fn is_conflicted(&self) -> bool {
        self.status == 'C' || self.tree_conflict == 'C'
    }
}

/// A single log entry from `svn log -v`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    pub revision: u64,
    pub author: String,
    pub date: String,
    /// Number of lines of the message
    pub line_count: u64,
    /// Changed paths: (action char, path)
    pub changed: Vec<(char, String)>,
    /// Full commit message
    pub message: String,
}

impl LogEntry {
    /// First non-empty line of the message (for the summary list).
    pub fn summary(&self) -> String {
        self.message
            .lines()
            .map(str::trim_end)
            .find(|l| !l.is_empty())
            .unwrap_or_default()
            .to_string()
    }
}

/// A single line of `svn blame` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    /// Revision number, or None for uncommitted lines ('-').
    pub revision: Option<u64>,
    pub author: String,
    pub content: String,
}

/// Diff content plus parsed hunk info for rendering with line numbers.
#[derive(Clone, Debug, Default)]
pub struct ParsedDiff {
    /// One line of the diff, fully styled-ready text (line numbers included).
    pub lines: Vec<DiffLine>,
}

/// A rendered diff line.
#[derive(Clone, Debug)]
pub struct DiffLine {
    /// Optional left (old) line number
    pub old: Option<u64>,
    /// Optional right (new) line number
    pub new: Option<u64>,
    /// Kind used for colorization
    pub kind: DiffLineKind,
    /// Raw content (without the leading + / - / space)
    pub content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Header,     // Index: / ==== lines
    FileHeader, // --- / +++ file headers
    Hunk,       // @@ -a,b +c,d @@
    Context,    // plain context line
    Added,      // +
    Removed,    // -
    Note,       // "\ No newline..." and other notices
}

/// Kind of item in the file tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeItemKind {
    Dir {
        expanded: bool,
    },
    File {
        /// Index into the raw status entry list
        entry: usize,
    },
}

/// A visible item of the file tree (flattened for rendering).
#[derive(Clone, Debug)]
pub struct TreeItem {
    pub depth: usize,
    pub path: String,
    pub name: String,
    pub kind: TreeItemKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svn_info_branch_label() {
        let info = SvnInfo {
            url: "file:///repo/trunk".into(),
            branch: "trunk".into(),
            revision: 42,
            wc_root: "/wc".into(),
        };
        assert_eq!(info.branch_label(), "trunk");
        let no_rel = SvnInfo {
            url: "file:///repo/trunk".into(),
            branch: String::new(),
            revision: 1,
            wc_root: String::new(),
        };
        assert_eq!(no_rel.branch_label(), "file:///repo/trunk");
    }

    #[test]
    fn conflicted_detection() {
        let mut e = StatusEntry {
            status: 'M',
            props_status: ' ',
            tree_conflict: ' ',
            path: "a".into(),
            is_dir: false,
        };
        assert!(!e.is_conflicted());
        e.status = 'C';
        assert!(e.is_conflicted());
        e.status = 'M';
        e.tree_conflict = 'C';
        assert!(e.is_conflicted());
    }

    #[test]
    fn log_summary_uses_first_line() {
        let e = LogEntry {
            revision: 3,
            author: "a".into(),
            date: "d".into(),
            line_count: 2,
            changed: vec![],
            message: "first\nsecond\n".into(),
        };
        assert_eq!(e.summary(), "first");
        let blank = LogEntry {
            message: "\n\nreal message\n".into(),
            ..e
        };
        assert_eq!(blank.summary(), "real message");
    }
}
