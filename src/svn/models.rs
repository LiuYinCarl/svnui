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
    /// Repository root URL ("Repository Root:")
    pub repo_root: String,
    /// Repository UUID
    pub uuid: String,
    /// "Last Changed Author:"
    pub last_author: String,
    /// "Last Changed Rev:"
    pub last_rev: u64,
    /// "Last Changed Date:" (verbatim, svn formats it localized)
    pub last_date: String,
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
    /// Directory-lock column (third): 'L' when the item is locked.
    pub locked: char,
    /// File-lock column (sixth): 'K' locked, 'O' other lock, 'T' stolen
    /// lock, 'B' broken lock; ' ' when unlocked.
    pub lock: char,
    /// Copied-with-history marker (fourth column), '+' when present.
    pub copied: char,
    /// Switched marker (fifth column), 'S' when present.
    pub switched: char,
    /// Path relative to the working-copy root (the app re-roots itself
    /// there after `svn info`)
    pub path: String,
    /// Whether the path is a directory on disk
    pub is_dir: bool,
}

impl Default for StatusEntry {
    fn default() -> Self {
        Self {
            status: ' ',
            props_status: ' ',
            tree_conflict: ' ',
            locked: ' ',
            lock: ' ',
            copied: ' ',
            switched: ' ',
            path: String::new(),
            is_dir: false,
        }
    }
}

impl StatusEntry {
    /// Text conflict (column 1), property conflict (column 2) or tree
    /// conflict (column 7)
    pub fn is_conflicted(&self) -> bool {
        self.status == 'C' || self.props_status == 'C' || self.tree_conflict == 'C'
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

/// A `svn diff` result parsed on the worker thread, ready to render.
///
/// Carrying the parsed form (instead of the raw text) stops big diffs from
/// being re-parsed on the UI thread (each line clones an owned `String`),
/// which used to freeze the app on large modified files.
#[derive(Clone, Debug)]
pub struct PreparedDiff {
    /// The parsed diff (empty when `empty_reason` is set).
    pub parsed: ParsedDiff,
    /// Shown instead of the diff when there is nothing to render (e.g.
    /// "No textual diff" for empty output).
    pub empty_reason: Option<String>,
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
    /// Set by the diff view (never by the parser): `Some` when this line
    /// belongs to a change that only touches whitespace.
    pub ws_only: Option<WsMark>,
}

/// What a whole-file view needs from the diff it was opened from, so the
/// file keeps the diff's colours: which file lines the diff adds, and where
/// its removed lines belong (they are inserted, making the view a
/// full-context diff).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileOverlay {
    /// New-side line numbers the diff adds (rendered as added lines).
    pub added: std::collections::HashSet<u64>,
    /// Removed lines `(old line number, content)` to insert *before* the
    /// given new-side line; [`u64::MAX`] means "after the last line".
    pub removed: std::collections::BTreeMap<u64, Vec<(u64, String)>>,
}

/// A whitespace-only change on one side of a removed/added pair, as byte
/// ranges into [`DiffLine::content`].
///
/// Whitespace is rendered as blanks (tabs are expanded), so the change is
/// expressed with background colours instead of marker glyphs: the
/// differing run gets the line's "changed whitespace" tint, and cells that
/// came from a tab get the tab tint — that is what keeps a tab
/// distinguishable from spaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsMark {
    /// Blank cells (spaces and tab expansions) of the differing run. Only
    /// blanks are tinted: a run may contain text (a blank moved across a
    /// word), and text keeps its readable foreground.
    pub blanks: Vec<(usize, usize)>,
    /// Tab-produced cells inside `blanks`.
    pub tabs: Vec<(usize, usize)>,
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

/// Details of a working-copy conflict, parsed from `svn info`.
///
/// svnui deliberately does not resolve conflicts; this is display-only
/// information so the user can see what kind of conflict they have and
/// take it to their editor / merge tool / `svn resolve`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictDetails {
    /// Working-copy-relative path (or the path the user asked about)
    pub path: String,
    /// Property names reported by "Conflicted Properties:"
    pub properties: Vec<String>,
    /// Conflict descriptions, e.g. "Conflict Details: incoming file edit
    /// upon update" or "Tree conflict: local edit, incoming delete..."
    pub summary: Vec<String>,
    /// Source revision lines ("Source left: ...", "Source right: ...")
    pub sources: Vec<String>,
}

impl ConflictDetails {
    pub fn is_empty(&self) -> bool {
        self.properties.is_empty() && self.summary.is_empty() && self.sources.is_empty()
    }
}

/// One entry from `svn status -u --xml`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteStatusEntry {
    /// Working-copy-relative path (XML-unescaped)
    pub path: String,
    /// `wc-status item` (normal/modified/added/deleted/missing/...)
    pub local_item: String,
    /// `wc-status props` (none/modified/conflicted)
    pub local_props: String,
    /// `repos-status item` when the server has a change
    pub remote_item: Option<String>,
    /// `repos-status props`
    pub remote_props: Option<String>,
}

/// Result of `svn status -u --xml`: what an update would bring in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteStatus {
    /// HEAD revision, when `svn info -r HEAD` succeeded.
    pub head_revision: Option<u64>,
    pub entries: Vec<RemoteStatusEntry>,
}

impl RemoteStatus {
    /// Entries the server reports as changed (added/modified/deleted/...).
    pub fn incoming(&self) -> impl Iterator<Item = &RemoteStatusEntry> {
        self.entries.iter().filter(|e| {
            e.remote_item.is_some()
                || !matches!(e.remote_props.as_deref(), None | Some("none") | Some(""))
        })
    }

    /// Entries with local changes (modified/added/deleted/missing/...).
    pub fn local(&self) -> impl Iterator<Item = &RemoteStatusEntry> {
        self.entries.iter().filter(|e| {
            !matches!(e.local_item.as_str(), "normal" | "none" | "")
                || !matches!(e.local_props.as_str(), "none" | "")
        })
    }
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
            repo_root: "file:///repo".into(),
            uuid: "u-1".into(),
            last_author: "alice".into(),
            last_rev: 40,
            last_date: "2026-01-01".into(),
        };
        assert_eq!(info.branch_label(), "trunk");
        let no_rel = SvnInfo {
            branch: String::new(),
            revision: 1,
            wc_root: String::new(),
            ..info.clone()
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
            ..Default::default()
        };
        assert!(!e.is_conflicted());
        e.status = 'C';
        assert!(e.is_conflicted());
        e.status = 'M';
        e.tree_conflict = 'C';
        assert!(e.is_conflicted());
    }

    #[test]
    fn props_conflict_is_conflicted() {
        // " C      f.txt": a property-only conflict sets column 2 and
        // leaves column 1 blank — it must still count as conflicted
        let e = StatusEntry {
            status: ' ',
            props_status: 'C',
            tree_conflict: ' ',
            path: "f.txt".into(),
            is_dir: false,
            ..Default::default()
        };
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
