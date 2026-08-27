//! Thin wrapper around the `svn` command line client.
//!
//! Mirrors the role of gitui's `asyncgit` crate: all SVN operations run on
//! background threads and report results through a channel, so the UI never
//! blocks on repository I/O.

pub mod models;
pub mod parser;

use crossbeam_channel::Sender;
use models::{BlameLine, LogEntry, StatusEntry, SvnInfo};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Result of a finished SVN background operation.
#[derive(Clone, Debug)]
pub enum AsyncSvnNotification {
    /// Working copy check at startup
    Info(Result<SvnInfo, String>),
    Status(Result<Vec<StatusEntry>, String>),
    Diff {
        path: String,
        result: Result<String, String>,
    },
    Log(Result<Vec<LogEntry>, String>),
    /// `svn log` restricted to a single file
    FileLog {
        path: String,
        result: Result<Vec<LogEntry>, String>,
    },
    /// All versioned files in the working copy (`svn list -R`)
    ListFiles(Result<Vec<String>, String>),
    RevisionDiff {
        revision: u64,
        result: Result<String, String>,
    },
    /// Combined diff of a revision range (`svn diff -r from-1:to`)
    RangeDiff {
        from: u64,
        to: u64,
        result: Result<String, String>,
    },
    Blame {
        path: String,
        result: Result<Vec<BlameLine>, String>,
    },
    Update(Result<String, String>),
    Commit(Result<String, String>),
    Add(Result<Vec<String>, String>),
    Revert(Result<Vec<String>, String>),
    Resolve(Result<String, String>),
    /// Working copy updated to a specific revision (svn update -r N)
    UpdateToRevision(Result<String, String>),
    /// Full-history search results (`svn log --search`)
    LogSearch(Result<Vec<LogEntry>, String>),
}

/// The SVN client. Cheap to clone (path + channel).
#[derive(Clone)]
pub struct Svn {
    cwd: PathBuf,
    tx: Sender<AsyncSvnNotification>,
}

impl Svn {
    pub fn new(cwd: PathBuf, tx: Sender<AsyncSvnNotification>) -> Self {
        Self { cwd, tx }
    }

    fn spawn<F>(&self, f: F)
    where
        F: FnOnce() -> AsyncSvnNotification + Send + 'static,
    {
        let tx = self.tx.clone();
        let _ = std::thread::Builder::new()
            .name("svn-worker".to_string())
            .spawn(move || {
                let _ = tx.send(f());
            });
    }

    // ----- async entry points -----

    /// Check whether cwd is a valid working copy (`svn info`) and return
    /// the parsed URL/branch/revision for display.
    pub fn check_info(&self) {
        let cwd = self.cwd.clone();
        self.spawn(move || {
            let result = Self::run_in(&cwd, &["info"]).map(|text| parser::parse_info(&text));
            AsyncSvnNotification::Info(result)
        });
    }

    pub fn status(&self) {
        let cwd = self.cwd.clone();
        self.spawn(move || {
            let result = Self::run_in(&cwd, &["status", "--ignore-externals"])
                .map(|text| parser::parse_status(&text, &cwd));
            AsyncSvnNotification::Status(result)
        });
    }

    pub fn diff(&self, path: &str) {
        let cwd = self.cwd.clone();
        let path = path.to_string();
        self.spawn(move || {
            // `svn diff` fails with E155010 for unversioned files, so fall
            // back to reading the file content in that case too.
            let diff_result = Self::run_in(&cwd, &["diff", "--", &path]);
            let result = match diff_result {
                Ok(out) if out.trim().is_empty() => Self::read_content_fallback(&cwd, &path),
                Ok(out) => Ok(out),
                Err(e) => {
                    if e.contains("E155010") {
                        Self::read_content_fallback(&cwd, &path)
                    } else {
                        Err(e)
                    }
                }
            };
            AsyncSvnNotification::Diff { path, result }
        });
    }

    fn read_content_fallback(cwd: &Path, path: &str) -> Result<String, String> {
        let full = cwd.join(path);
        match std::fs::metadata(&full) {
            Ok(m) if m.len() > 2_000_000 => Ok(String::new()),
            Ok(_) => {
                std::fs::read_to_string(&full).map_err(|e| format!("failed to read {path}: {e}"))
            }
            Err(_) => Ok(String::new()),
        }
    }

    pub fn log(&self, limit: usize) {
        let cwd = self.cwd.clone();
        self.spawn(move || {
            // HEAD:0 so an empty (r0) repository yields no output instead
            // of an E160006 error
            let result = Self::run_in(
                &cwd,
                &["log", "-v", "-r", "HEAD:0", "-l", &limit.to_string()],
            )
            .map(|out| parser::parse_log(&out));
            AsyncSvnNotification::Log(result)
        });
    }

    /// `svn log` restricted to a single file (its history).
    pub fn file_log(&self, path: &str, limit: usize) {
        let cwd = self.cwd.clone();
        let path = path.to_string();
        self.spawn(move || {
            let result = Self::run_in(
                &cwd,
                &[
                    "log",
                    "-v",
                    "-r",
                    "HEAD:0",
                    "-l",
                    &limit.to_string(),
                    "--",
                    &path,
                ],
            )
            .map(|out| parser::parse_log(&out));
            AsyncSvnNotification::FileLog { path, result }
        });
    }

    /// Search the full commit history with `svn log --search` (matches
    /// author, date, message and changed paths case-sensitively; the
    /// option exists since svn 1.8). A larger limit than the default view
    /// is used so search can reach further back.
    pub fn log_search(&self, pattern: &str) {
        let cwd = self.cwd.clone();
        let pattern = pattern.to_string();
        self.spawn(move || {
            let result = Self::run_in(
                &cwd,
                &[
                    "log", "-v", "-r", "HEAD:0", "-l", "200", "--search", &pattern,
                ],
            )
            .map(|out| parser::parse_log(&out));
            AsyncSvnNotification::LogSearch(result)
        });
    }

    /// List all versioned files in the working copy (`svn list -R`).
    /// Directory entries (trailing '/') are filtered out.
    ///
    /// The `.@HEAD` peg is required: plain `svn list -R` lists the wc root
    /// at its BASE revision, which in a mixed-revision working copy (e.g.
    /// right after a commit, before update) can be stale or even empty.
    pub fn list_files(&self) {
        let cwd = self.cwd.clone();
        self.spawn(move || {
            let result = Self::run_in(&cwd, &["list", "-R", ".@HEAD"]).map(|out| {
                out.lines()
                    .map(|l| l.trim_end_matches('\r'))
                    .filter(|l| !l.is_empty() && !l.ends_with('/'))
                    .map(str::to_string)
                    .collect()
            });
            AsyncSvnNotification::ListFiles(result)
        });
    }

    pub fn revision_diff(&self, revision: u64) {
        let cwd = self.cwd.clone();
        self.spawn(move || {
            let result = Self::run_in(&cwd, &["diff", "-c", &revision.to_string()]);
            AsyncSvnNotification::RevisionDiff { revision, result }
        });
    }

    /// Combined diff of the revisions `from..=to` (`svn diff -r from-1:to`).
    pub fn range_diff(&self, from: u64, to: u64) {
        let cwd = self.cwd.clone();
        self.spawn(move || {
            let range = format!("{}:{to}", from.saturating_sub(1));
            let result = Self::run_in(&cwd, &["diff", "-r", &range]);
            AsyncSvnNotification::RangeDiff { from, to, result }
        });
    }

    pub fn blame(&self, path: &str) {
        let cwd = self.cwd.clone();
        let path = path.to_string();
        self.spawn(move || {
            let result =
                Self::run_in(&cwd, &["blame", "--", &path]).map(|out| parser::parse_blame(&out));
            AsyncSvnNotification::Blame { path, result }
        });
    }

    pub fn add(&self, paths: &[String]) {
        let cwd = self.cwd.clone();
        let paths = paths.to_vec();
        self.spawn(move || {
            let mut args: Vec<String> = vec!["add".into(), "--".into()];
            args.extend(paths.iter().cloned());
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let result = Self::run_in(&cwd, &arg_refs).map(|_| paths.clone());
            AsyncSvnNotification::Add(result)
        });
    }

    pub fn revert(&self, paths: &[String]) {
        let cwd = self.cwd.clone();
        let paths = paths.to_vec();
        self.spawn(move || {
            let mut args: Vec<String> = vec!["revert".into(), "-R".into(), "--".into()];
            args.extend(paths.iter().cloned());
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let result = Self::run_in(&cwd, &arg_refs).map(|_| paths.clone());
            AsyncSvnNotification::Revert(result)
        });
    }

    pub fn resolve(&self, path: &str) {
        let cwd = self.cwd.clone();
        let path = path.to_string();
        self.spawn(move || {
            let result = Self::run_in(&cwd, &["resolve", "--accept=working", "--", &path]);
            AsyncSvnNotification::Resolve(result.map(|_| path.clone()))
        });
    }

    pub fn update(&self) {
        let cwd = self.cwd.clone();
        self.spawn(move || {
            let result = Self::run_in(&cwd, &["update"]);
            AsyncSvnNotification::Update(result)
        });
    }

    pub fn update_to_revision(&self, revision: u64) {
        let cwd = self.cwd.clone();
        self.spawn(move || {
            let result = Self::run_in(&cwd, &["update", "-r", &revision.to_string()]);
            AsyncSvnNotification::UpdateToRevision(result)
        });
    }

    pub fn commit(&self, message: &str, paths: &[String]) {
        let cwd = self.cwd.clone();
        let message = message.to_string();
        let paths = paths.to_vec();
        self.spawn(move || {
            // Explicit file targets are committed exactly as listed (the
            // obsolete -N/--non-recursive flag is deliberately not used: it
            // is a no-op for file targets and may disappear in future svn).
            let mut args: Vec<String> = vec!["commit".into()];
            // The message is UTF-8; say so explicitly, otherwise svn
            // interprets -m in the "native" encoding, which is ASCII under
            // the LC_ALL=C forced by run_in, and non-ASCII messages are
            // rejected with E000022 ("Can't convert string ... to 'UTF-8'").
            args.push("--encoding".into());
            args.push("UTF-8".into());
            args.push("-m".into());
            args.push(message);
            args.push("--".into());
            args.extend(paths.iter().cloned());
            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let result = Self::run_in(&cwd, &arg_refs);
            AsyncSvnNotification::Commit(result)
        });
    }

    // ----- static helpers used by worker threads -----

    fn run_in(cwd: &Path, args: &[&str]) -> Result<String, String> {
        let out = Command::new("svn")
            .arg("--non-interactive")
            // Keep svn's messages English (the parsers match "Changed
            // paths:", "Index:", the status columns, ...) without forcing
            // the C *encoding*: LC_ALL=C would make ASCII the native
            // encoding, escaping non-ASCII log output as {U+XXXX} and
            // rejecting non-ASCII -m messages. LC_MESSAGES only selects
            // the gettext catalog; the codeset still follows the user's
            // locale (LC_CTYPE/LANG). Drop any inherited LC_ALL, which
            // would override LC_MESSAGES.
            .env_remove("LC_ALL")
            .env("LC_MESSAGES", "C")
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|e| format!("failed to run svn: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(err.trim().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self, TestRepo};
    use crossbeam_channel::unbounded;
    use std::time::Duration;

    fn recv<T>(rx: &crossbeam_channel::Receiver<T>) -> T {
        rx.recv_timeout(Duration::from_secs(15))
            .expect("timed out waiting for async svn result")
    }

    #[test]
    fn info_ok_and_err() {
        let Some(repo) = TestRepo::new() else { return };
        let (tx, rx) = unbounded();
        let c = Svn::new(repo.wc.clone(), tx);
        c.check_info();
        match recv(&rx) {
            AsyncSvnNotification::Info(Ok(info)) => {
                assert!(info.url.contains("file://"), "{info:?}");
                // NB: the wc *root dir* revision stays at its BASE value in
                // a mixed-revision working copy, so no assertion on it here
            }
            other => panic!("unexpected: {other:?}"),
        }

        // not a working copy
        let dir = std::env::temp_dir().join(format!("svnui-nonwc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx2, rx2) = unbounded();
        let c2 = Svn::new(dir.clone(), tx2);
        c2.check_info();
        assert!(matches!(recv(&rx2), AsyncSvnNotification::Info(Err(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_lists_changes() {
        let Some(repo) = TestRepo::new() else { return };
        test_support::write_file(&repo.wc.join("new.txt"), "new\n");
        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 2\n");
        let (tx, rx) = unbounded();
        Svn::new(repo.wc.clone(), tx).status();
        match recv(&rx) {
            AsyncSvnNotification::Status(Ok(entries)) => {
                let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
                assert!(paths.contains(&"new.txt"), "{paths:?}");
                assert!(paths.contains(&"Cargo.toml"));
                let new = entries.iter().find(|e| e.path == "new.txt").unwrap();
                assert_eq!(new.status, '?');
                let cargo = entries.iter().find(|e| e.path == "Cargo.toml").unwrap();
                assert_eq!(cargo.status, 'M');
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn diff_modified_and_unversioned() {
        let Some(repo) = TestRepo::new() else { return };
        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 3\n");
        test_support::write_file(&repo.wc.join("untracked.txt"), "raw content\n");
        let (tx, rx) = unbounded();
        let c = Svn::new(repo.wc.clone(), tx);
        c.diff("Cargo.toml");
        match recv(&rx) {
            AsyncSvnNotification::Diff { path, result } => {
                assert_eq!(path, "Cargo.toml");
                let content = result.unwrap();
                assert!(content.contains("Index: Cargo.toml"));
                assert!(content.contains("version = 3"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        c.diff("untracked.txt");
        match recv(&rx) {
            AsyncSvnNotification::Diff { path, result } => {
                assert_eq!(path, "untracked.txt");
                // fallback reads file content
                assert_eq!(result.unwrap().trim(), "raw content");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn log_lists_revisions() {
        let Some(repo) = TestRepo::new() else { return };
        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 2\n");
        repo.svn(&["commit", "-m", "second"]);
        let (tx, rx) = unbounded();
        Svn::new(repo.wc.clone(), tx).log(50);
        match recv(&rx) {
            AsyncSvnNotification::Log(Ok(entries)) => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].revision, 2);
                assert_eq!(entries[0].message, "second");
                assert_eq!(entries[1].revision, 1);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn revision_diff_and_blame() {
        let Some(repo) = TestRepo::new() else { return };
        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 5\n");
        repo.svn(&["commit", "-m", "bump"]);
        let (tx, rx) = unbounded();
        let c = Svn::new(repo.wc.clone(), tx);
        c.revision_diff(2);
        match recv(&rx) {
            AsyncSvnNotification::RevisionDiff { revision, result } => {
                assert_eq!(revision, 2);
                let content = result.unwrap();
                assert!(content.contains("Index: Cargo.toml"));
            }
            other => panic!("unexpected: {other:?}"),
        }
        c.blame("src/main.rs");
        match recv(&rx) {
            AsyncSvnNotification::Blame { path, result } => {
                assert_eq!(path, "src/main.rs");
                let lines = result.unwrap();
                assert!(!lines.is_empty());
                assert!(lines[0].revision.is_some());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn add_revert_resolve_commit_update_flow() {
        let Some(repo) = TestRepo::new() else { return };
        let (tx, rx) = unbounded();
        let c = Svn::new(repo.wc.clone(), tx);

        // add an unversioned file
        test_support::write_file(&repo.wc.join("added.txt"), "x\n");
        c.add(&["added.txt".to_string()]);
        match recv(&rx) {
            AsyncSvnNotification::Add(Ok(paths)) => assert_eq!(paths, vec!["added.txt"]),
            other => panic!("unexpected: {other:?}"),
        }
        assert!(repo.svn(&["status"]).contains("A"));

        // revert it: un-adds, file remains as unversioned
        c.revert(&["added.txt".to_string()]);
        match recv(&rx) {
            AsyncSvnNotification::Revert(Ok(paths)) => assert_eq!(paths, vec!["added.txt"]),
            other => panic!("unexpected: {other:?}"),
        }
        let status = repo.svn(&["status"]);
        assert!(
            status.contains("?"),
            "expected unversioned marker: {status:?}"
        );

        // modify + commit
        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 9\n");
        c.commit("bump version", &["Cargo.toml".to_string()]);
        match recv(&rx) {
            AsyncSvnNotification::Commit(Ok(out)) => {
                assert!(out.contains("Committed revision 2"), "{out}");
            }
            other => panic!("unexpected: {other:?}"),
        }
        // commit with empty paths = all changes
        test_support::write_file(&repo.wc.join("docs/readme.md"), "# docs v2\n");
        c.commit("update the readme", &[]);
        match recv(&rx) {
            AsyncSvnNotification::Commit(Ok(out)) => {
                assert!(out.contains("Committed revision 3"), "{out}");
            }
            other => panic!("unexpected: {other:?}"),
        }

        // update: no-op, still succeeds
        c.update();
        match recv(&rx) {
            AsyncSvnNotification::Update(Ok(out)) => {
                assert!(out.contains("At revision 3"), "{out}");
            }
            other => panic!("unexpected: {other:?}"),
        }

        // update to an older revision
        c.update_to_revision(1);
        match recv(&rx) {
            AsyncSvnNotification::UpdateToRevision(Ok(_)) => {}
            other => panic!("unexpected: {other:?}"),
        }
        assert!(repo.svn(&["info"]).contains("Revision: 1"));
        c.update_to_revision(3);
        assert!(matches!(
            recv(&rx),
            AsyncSvnNotification::UpdateToRevision(Ok(_))
        ));

        // resolve on a non-conflicted file still exits 0
        c.resolve("Cargo.toml");
        match recv(&rx) {
            AsyncSvnNotification::Resolve(Ok(path)) => assert_eq!(path, "Cargo.toml"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// Non-ASCII (CJK) log messages must survive a round trip: the forced
    /// LC_MESSAGES=C pins English keywords but leaves the UTF-8 codeset,
    /// and commit passes --encoding UTF-8 so -m is not misread as ASCII.
    /// Regression test for E000022 "Can't convert string ... to 'UTF-8'"
    /// and for log output showing "{U+6D4B}..." escapes.
    #[test]
    fn commit_and_log_cjk_message() {
        let Some(repo) = TestRepo::new() else { return };
        let (tx, rx) = unbounded();
        let c = Svn::new(repo.wc.clone(), tx);

        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 2\n");
        c.commit("测试一下提交吧", &["Cargo.toml".to_string()]);
        match recv(&rx) {
            AsyncSvnNotification::Commit(Ok(out)) => {
                assert!(out.contains("Committed revision 2"), "{out}");
            }
            other => panic!("unexpected: {other:?}"),
        }

        c.log(1);
        match recv(&rx) {
            AsyncSvnNotification::Log(Ok(entries)) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].message, "测试一下提交吧");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn diff_missing_file_falls_back_to_empty() {
        let Some(repo) = TestRepo::new() else { return };
        let (tx, rx) = unbounded();
        Svn::new(repo.wc.clone(), tx).diff("does-not-exist.txt");
        match recv(&rx) {
            AsyncSvnNotification::Diff {
                result: Ok(content),
                ..
            } => {
                assert!(content.is_empty());
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn file_log_lists_only_touching_revisions() {
        let Some(repo) = TestRepo::new() else { return };
        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 2\n");
        repo.svn(&["commit", "-m", "bump cargo"]);
        let (tx, rx) = unbounded();
        let c = Svn::new(repo.wc.clone(), tx);
        c.file_log("Cargo.toml", 50);
        match recv(&rx) {
            AsyncSvnNotification::FileLog { path, result } => {
                assert_eq!(path, "Cargo.toml");
                let entries = result.unwrap();
                // both r1 (add) and r2 (bump) touched Cargo.toml
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].revision, 2);
                assert_eq!(entries[0].message, "bump cargo");
            }
            other => panic!("unexpected: {other:?}"),
        }
        // error path: unversioned file
        c.file_log("no-such-file.txt", 50);
        assert!(matches!(
            recv(&rx),
            AsyncSvnNotification::FileLog { result: Err(_), .. }
        ));
    }

    #[test]
    fn list_files_returns_versioned_files_only() {
        let Some(repo) = TestRepo::new() else { return };
        test_support::write_file(&repo.wc.join("unversioned.txt"), "x\n");
        let (tx, rx) = unbounded();
        Svn::new(repo.wc.clone(), tx).list_files();
        match recv(&rx) {
            AsyncSvnNotification::ListFiles(Ok(files)) => {
                assert!(files.contains(&"Cargo.toml".to_string()), "{files:?}");
                assert!(files.contains(&"src/main.rs".to_string()), "{files:?}");
                assert!(files.contains(&"docs/readme.md".to_string()), "{files:?}");
                // dirs are stripped, unversioned files absent
                assert!(!files.iter().any(|f| f.ends_with('/')));
                assert!(!files.contains(&"unversioned.txt".to_string()));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn range_diff_combines_revisions() {
        let Some(repo) = TestRepo::new() else { return };
        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 2\n");
        repo.svn(&["commit", "-m", "r2"]);
        test_support::write_file(&repo.wc.join("Cargo.toml"), "version = 3\n");
        repo.svn(&["commit", "-m", "r3"]);
        let (tx, rx) = unbounded();
        let c = Svn::new(repo.wc.clone(), tx);
        c.range_diff(2, 3);
        match recv(&rx) {
            AsyncSvnNotification::RangeDiff { from, to, result } => {
                assert_eq!((from, to), (2, 3));
                let content = result.unwrap();
                assert!(content.contains("Index: Cargo.toml"), "{content}");
                // combined diff goes straight from r1 content to r3 content
                assert!(content.contains("-version = 1"), "{content}");
                assert!(content.contains("+version = 3"), "{content}");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn revision_diff_error_reports_stderr() {
        let Some(repo) = TestRepo::new() else { return };
        let (tx, rx) = unbounded();
        Svn::new(repo.wc.clone(), tx).revision_diff(999_999);
        match recv(&rx) {
            AsyncSvnNotification::RevisionDiff { result: Err(e), .. } => {
                assert!(!e.is_empty(), "expected an error message");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
