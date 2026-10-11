//! `.svnignore` support: gitignore-style filtering of `svn status` entries.
//!
//! Large working copies often contain generated or vendor trees the user
//! never wants to see in the status view. A `.svnignore` file at the
//! working-copy root filters those entries out of the tree using
//! .gitignore syntax (`*` / `**` globs, `dir/` directory-only patterns,
//! `/`-anchored patterns, `!` negation, `#` comments). Matching is done
//! by the `ignore` crate (ripgrep's gitignore engine) — the semantics
//! are too subtle to hand-roll.
//!
//! Hidden entries are *only* a display filter: they stay untouched on
//! disk and in svn, and can neither be staged nor committed from the TUI
//! while hidden.

use ignore::gitignore::Gitignore;
use std::borrow::Cow;
use std::path::Path;

/// Name of the ignore file at the working-copy root.
pub const IGNORE_FILE: &str = ".svnignore";

/// A loaded `.svnignore` rule set.
pub struct IgnoreFilter {
    matcher: Gitignore,
    /// Extra path prefix (relative to the ignore root) prepended to
    /// status-relative paths before matching — non-empty when the app was
    /// launched from a subdirectory of the working-copy root and the
    /// `.svnignore` lives at the root.
    prefix: String,
}

impl IgnoreFilter {
    /// Load `<root>/.svnignore`. Returns `None` when the file does not
    /// exist or contains no effective ignore rules (a file with only
    /// comments, blanks or lone `!` negations ignores nothing).
    ///
    /// Unparseable lines are dropped by the underlying parser; the valid
    /// remainder still applies.
    pub fn load(root: &Path) -> Option<Self> {
        Self::load_for(root, root)
    }

    /// Load `<root>/.svnignore` whose patterns are relative to `root`,
    /// while the status entries being filtered are relative to
    /// `status_base`. When the app was launched from a subdirectory of the
    /// working copy, the status paths are prefixed accordingly so root-
    /// anchored patterns like `/build/` still match.
    pub fn load_for(root: &Path, status_base: &Path) -> Option<Self> {
        let path = root.join(IGNORE_FILE);
        if !path.is_file() {
            return None;
        }
        let (matcher, _err) = Gitignore::new(&path);
        if matcher.num_ignores() == 0 {
            return None;
        }
        let prefix = status_base
            .strip_prefix(root)
            .ok()
            .and_then(|rel| rel.to_str().map(str::to_string))
            .map(|s| s.replace('\\', "/"));
        let Some(prefix) = prefix else {
            // `status_base` is not under `root` (or is non-UTF-8): matching
            // root patterns against status-relative paths would be wrong
            // (anchored patterns would silently stop matching). Fall back
            // to the long-standing cwd-relative semantics instead.
            return Self::load_for(status_base, status_base);
        };
        Some(Self { matcher, prefix })
    }

    /// Whether a working-copy-relative path is ignored. Like gitignore, a
    /// path is ignored when any of its parent directories is ignored too.
    pub fn is_ignored(&self, path: &str, is_dir: bool) -> bool {
        let joined: Cow<str> = if self.prefix.is_empty() {
            Cow::Borrowed(path)
        } else {
            Cow::Owned(format!("{}/{path}", self.prefix))
        };
        self.matcher
            .matched_path_or_any_parents(joined.as_ref(), is_dir)
            .is_ignore()
    }

    /// Split status entries into (visible, hidden) by the ignore rules.
    pub fn partition<T>(
        &self,
        entries: Vec<T>,
        path_of: impl Fn(&T) -> &str,
        is_dir_of: impl Fn(&T) -> bool,
    ) -> (Vec<T>, Vec<T>) {
        entries
            .into_iter()
            .partition(|e| !self.is_ignored(path_of(e), is_dir_of(e)))
    }

    /// Build a filter from in-memory rules (tests only; production loads
    /// from the working-copy root via [`IgnoreFilter::load`]).
    #[cfg(test)]
    pub fn from_lines(lines: &str) -> Option<Self> {
        let mut builder = ignore::gitignore::GitignoreBuilder::new("");
        for line in lines.lines() {
            let _ = builder.add_line(None, line);
        }
        let matcher = builder.build().ok()?;
        (matcher.num_ignores() > 0).then(|| Self {
            matcher,
            prefix: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A temp dir holding the given `.svnignore` content, cleaned up on drop.
    struct IgnoreDir(std::path::PathBuf);

    impl IgnoreDir {
        fn new(content: Option<&str>) -> Self {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("svnui-ignore-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            if let Some(content) = content {
                std::fs::write(dir.join(IGNORE_FILE), content).unwrap();
            }
            Self(dir)
        }

        fn filter(&self) -> Option<IgnoreFilter> {
            IgnoreFilter::load(&self.0)
        }
    }

    impl Drop for IgnoreDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn load_for_prefixes_status_paths_to_the_ignore_root() {
        let root = IgnoreDir::new(Some("build/\n/only_top/\n"));
        let at_root = root.filter().expect("root ignore file");
        assert!(at_root.is_ignored("build/x.rs", false));
        assert!(at_root.is_ignored("only_top/y.rs", false));

        // launched in a subdirectory: status entries are subdir-relative,
        // so load_for prepends the subdir prefix before matching —
        // unanchored patterns still hit, anchored ones only at the root
        let sub = root.0.join("src");
        let in_sub = IgnoreFilter::load_for(&root.0, &sub).expect("root ignore file");
        assert!(in_sub.is_ignored("build/x.rs", false));
        assert!(!in_sub.is_ignored("only_top/y.rs", false));

        // status_base not under root: no prefix is computable, so the
        // filter falls back to status_base-relative semantics (a plain
        // load of that dir, which has no .svnignore)
        let outside = std::env::temp_dir().join("svnui-outside-root");
        assert!(IgnoreFilter::load_for(&root.0, &outside).is_none());
    }

    #[test]
    fn missing_or_empty_file_means_no_filter() {
        let d = IgnoreDir::new(None);
        assert!(d.filter().is_none());
        let d = IgnoreDir::new(Some(""));
        assert!(d.filter().is_none());
        // comments, blanks and lone negations ignore nothing
        let d = IgnoreDir::new(Some("# comment\n\n!important.log\n"));
        assert!(d.filter().is_none());
    }

    #[test]
    fn basename_glob_matches_at_any_depth() {
        let d = IgnoreDir::new(Some("*.log\n"));
        let f = d.filter().unwrap();
        assert!(f.is_ignored("build.log", false));
        assert!(f.is_ignored("deep/nested/error.log", false));
        assert!(!f.is_ignored("src/main.rs", false));
    }

    #[test]
    fn dir_pattern_hides_the_whole_tree() {
        let d = IgnoreDir::new(Some("target/\n"));
        let f = d.filter().unwrap();
        assert!(f.is_ignored("target", true));
        // files below an ignored dir are ignored via their parents
        assert!(f.is_ignored("target/debug/app.o", false));
        // a *file* named target is not matched by a dir-only pattern
        assert!(!f.is_ignored("target", false));
        assert!(!f.is_ignored("src/targets.txt", false));
    }

    #[test]
    fn anchored_and_double_star_patterns() {
        let d = IgnoreDir::new(Some("/dist\n**/generated/**\ndocs/**/*.html\n"));
        let f = d.filter().unwrap();
        assert!(f.is_ignored("dist", true));
        assert!(f.is_ignored("dist/app.js", false));
        // anchored: a "dist" deeper down is not matched
        assert!(!f.is_ignored("packages/dist", true));
        // a trailing `/**` matches everything *inside*, not the dir itself
        // (git semantics; use `generated/` to hide the dir as well)
        assert!(!f.is_ignored("a/generated", true));
        assert!(f.is_ignored("a/b/generated/c/d.rs", false));
        assert!(f.is_ignored("docs/api/x.html", false));
        assert!(!f.is_ignored("web/x.html", false));
    }

    #[test]
    fn negation_reincludes() {
        let d = IgnoreDir::new(Some("*.log\n!keep.log\n"));
        let f = d.filter().unwrap();
        assert!(f.is_ignored("error.log", false));
        assert!(!f.is_ignored("keep.log", false));
        assert!(!f.is_ignored("sub/keep.log", false));
    }

    #[test]
    fn invalid_lines_do_not_void_the_rest() {
        // a dangling backslash is a parse error; the bad line is dropped
        // by the parser and the remaining rules still apply
        let d = IgnoreDir::new(Some("foo\\\n*.tmp\n"));
        let f = d.filter().unwrap();
        assert!(f.is_ignored("x.tmp", false));
        // NB: "[bad" is NOT invalid — gitignore treats an unclosed '[' as
        // literal text, so do not use it as the bad-line fixture
    }

    #[test]
    fn partition_splits_entries() {
        let d = IgnoreDir::new(Some("vendor/\n*.log\n"));
        let f = d.filter().unwrap();
        let entries = vec![
            ("src/main.rs", false),
            ("vendor", true),
            ("vendor/lib/a.c", false),
            ("build.log", false),
        ];
        let (vis, hid) = f.partition(entries, |e| e.0, |e| e.1);
        assert_eq!(vis, vec![("src/main.rs", false)]);
        assert_eq!(hid.len(), 3);
    }
}
