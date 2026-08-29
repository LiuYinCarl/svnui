//! Test support: temp SVN repositories and helpers.
//!
//! Compiled unconditionally so that both the library tests and the binary's
//! tests can use it; it is only referenced from `#[cfg(test)]` code.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary SVN repository + working copy, cleaned up on drop.
pub struct TestRepo {
    dir: PathBuf,
    pub repo: PathBuf,
    pub wc: PathBuf,
}

impl TestRepo {
    /// Create a fresh repo with an initial layout committed.
    ///
    /// Returns None if the svn binaries are unavailable (test is skipped).
    pub fn new() -> Option<Self> {
        if Command::new("svn").arg("--version").output().is_err()
            || Command::new("svnadmin").arg("--version").output().is_err()
        {
            return None;
        }
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("svnui-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let repo = dir.join("repo");
        let wc = dir.join("wc");
        if !run("svnadmin", &["create", repo.to_str().unwrap()]) {
            return None;
        }
        if !run(
            "svn",
            &[
                "co",
                format!("file://{}", repo.display()).as_str(),
                wc.to_str().unwrap(),
            ],
        ) {
            return None;
        }
        let repo = Self { dir, repo, wc };
        repo.seed();
        Some(repo)
    }

    /// Initial committed layout: src/main.rs, Cargo.toml, docs/readme.md.
    fn seed(&self) {
        write_file(
            &self.wc.join("src/main.rs"),
            "fn main() {\n    println!(\"hi\");\n}\n",
        );
        write_file(&self.wc.join("Cargo.toml"), "version = 1\n");
        write_file(&self.wc.join("docs/readme.md"), "# docs\n");
        svn(&self.wc, &["add", "src", "docs", "Cargo.toml"]);
        svn(&self.wc, &["commit", "-m", "initial commit"]);
    }

    /// Run a command in the working copy, returning stdout.
    pub fn svn(&self, args: &[&str]) -> String {
        svn(&self.wc, args)
    }
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Write a file, creating parent dirs.
pub fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, content).expect("write file");
}

/// Run `svn` in a directory; returns stdout. Panics on failure.
///
/// Like the app's `run_in`, this pins English messages (LC_MESSAGES=C) so
/// assertions on svn output hold regardless of the test runner's locale,
/// while leaving the locale codeset (UTF-8) intact for non-ASCII data.
pub fn svn(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("svn")
        .args(args)
        .env_remove("LC_ALL")
        .env("LC_MESSAGES", "C")
        .current_dir(cwd)
        .output()
        .expect("run svn");
    assert!(
        out.status.success(),
        "svn {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Run an arbitrary command; returns success.
fn run(prog: &str, args: &[&str]) -> bool {
    Command::new(prog)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Render a closure into a TestBackend terminal and return it.
pub fn render<F>(width: u16, height: u16, f: F) -> ratatui::Terminal<ratatui::backend::TestBackend>
where
    F: FnOnce(&mut ratatui::Frame),
{
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("test terminal");
    terminal.draw(f).expect("draw");
    terminal
}

/// Dump the full buffer of a terminal as a string (for assertions).
pub fn dump(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
    let buf = terminal.backend().buffer();
    let mut out = String::new();
    let area = buf.area();
    let (w, h) = (area.width, area.height);
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

/// A crossterm key event helper.
pub fn key(code: crossterm::event::KeyCode) -> crossterm::event::Event {
    crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
        code,
        crossterm::event::KeyModifiers::NONE,
    ))
}

/// Generate synthetic status entries for performance tests.
///
/// `wide`: `n` files in a single directory (worst case for tree building);
/// `deep`: files nested under shared directories (more realistic layout).
pub fn gen_status_entries(n: usize, wide: bool) -> Vec<crate::svn::models::StatusEntry> {
    use crate::svn::models::StatusEntry;
    (0..n)
        .map(|i| {
            let path = if wide {
                format!("file_{i:06}.rs")
            } else {
                format!("src/d{}/d{}/d{}/mod_{i:06}.rs", i % 40, i % 20, i % 10)
            };
            StatusEntry {
                status: if i % 3 == 0 { 'M' } else { '?' },
                props_status: ' ',
                tree_conflict: ' ',
                path,
                is_dir: false,
            }
        })
        .collect()
}
