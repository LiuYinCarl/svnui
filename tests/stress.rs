//! Headless stress test: drives the real `App` against a large SVN working
//! copy with a deterministic stream of synthetic crossterm key events.
//!
//! The working copy is produced by `scripts/stress_test.sh`, which converts
//! a real git repository (default: openless) into an SVN repo via git2svn
//! and checks out `target/tmp/stress/wc`.
//!
//! This test is inert unless explicitly enabled — CI never sets the env
//! vars, so it skips (passes) there:
//!
//! ```sh
//! SVNUI_STRESS=1 SVNUI_STRESS_WC=<working copy> \
//!     cargo test --test stress -- --nocapture --test-threads=1
//! ```
//!
//! Knobs:
//! - `SVNUI_STRESS_ROUNDS` — number of randomized rounds (default 200)
//! - `SVNUI_STRESS_SEED`  — PRNG seed (default fixed); printed at start and
//!   included in every failure message so a failing run is reproducible

use crossbeam_channel::{Receiver, unbounded};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use svnui::app::App;
use svnui::components::Context;
use svnui::popups::Popup;
use svnui::queue::Queue;
use svnui::svn::{self, Svn};
use svnui::ui::style::Theme;

/// Generous bound for draining all pending svn operations of one round.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(120);

/// xorshift64* — tiny deterministic PRNG so a run is reproducible from its
/// seed (no rand crate).
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[derive(Default)]
struct Stats {
    commits: usize,
    reverts: usize,
    diffs: usize,
    blames: usize,
    searches: usize,
    patches: usize,
    refreshes: usize,
    updates: usize,
    benign_errors: usize,
    anomalies: usize,
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "commits={} reverts={} diffs={} blames={} searches={} patches={} \
             refreshes={} updates={} benign_errors={} anomalies={}",
            self.commits,
            self.reverts,
            self.diffs,
            self.blames,
            self.searches,
            self.patches,
            self.refreshes,
            self.updates,
            self.benign_errors,
            self.anomalies,
        )
    }
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn ctrl(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}

/// Error popup substrings that are tolerated (logged and counted) instead
/// of failing the run — e.g. `svn blame` on a binary file that slipped
/// past the extension filter.
fn is_benign_error(msg: &str) -> bool {
    msg.contains("not under version control") || msg.contains("binary")
}

/// Files that are very likely binary (blame/diff on them is not useful).
fn is_binary_name(path: &str) -> bool {
    const EXT: [&str; 20] = [
        "png", "jpg", "jpeg", "gif", "ico", "icns", "webp", "woff", "woff2", "ttf", "otf", "eot",
        "pdf", "zip", "gz", "mp3", "mp4", "mov", "wasm", "jar",
    ];
    let lower = path.to_lowercase();
    lower.rsplit('.').next().is_some_and(|e| EXT.contains(&e))
}

/// Append one line to a working-copy file.
fn append_line(path: &Path, line: &str) {
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap_or_else(|e| panic!("cannot open {}: {e}", path.display()));
    writeln!(f, "{line}").expect("append line");
}

/// All versioned text files of the working copy (`svn list -R .@HEAD`).
fn list_wc_files(wc: &Path) -> Vec<String> {
    let out = svnui::test_support::svn(wc, &["list", "-R", ".@HEAD"]);
    out.lines()
        .filter(|l| !l.is_empty() && !l.ends_with('/'))
        .filter(|l| !is_binary_name(l))
        .map(|l| l.to_string())
        .collect()
}

/// The stress harness: a real `App` pumped exactly like main.rs's run loop
/// (input → handle_queue_events → maybe_request_diff; async notification →
/// handle_async → handle_queue_events → maybe_request_diff), plus a
/// TestBackend draw after each action to exercise rendering.
struct Harness {
    app: App,
    rx: Receiver<svn::AsyncSvnNotification>,
    wc: PathBuf,
    files: Vec<String>,
    rng: XorShift,
    seed: u64,
    round: usize,
    action: &'static str,
    stats: Stats,
    /// Current text of the status-tree filter (its field is private, so we
    /// mirror it here to be able to clear the filter again).
    tree_filter: String,
}

impl Harness {
    fn fail(&self, msg: &str) -> ! {
        panic!(
            "STRESS FAILURE (seed={}, round={}, action={}): {msg}",
            self.seed, self.round, self.action
        )
    }

    /// One input event through the same pump as main.rs's run loop.
    fn input(&mut self, ev: Event) {
        if let Err(e) = self.app.handle_input(&ev) {
            self.fail(&format!("handle_input: {e}"));
        }
        self.app.handle_queue_events();
        self.app.maybe_request_diff();
    }

    fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.input(key(KeyCode::Char(c)));
        }
    }

    /// Drain async notifications until no svn operation is pending.
    fn settle(&mut self) {
        let deadline = Instant::now() + SETTLE_TIMEOUT;
        loop {
            self.app.handle_queue_events();
            self.app.maybe_request_diff();
            self.app.handle_queue_events();
            if self.app.pending == 0 {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.fail(&format!(
                    "pending={} did not drain within {SETTLE_TIMEOUT:?}",
                    self.app.pending
                ));
            }
            match self.rx.recv_timeout(remaining) {
                Ok(notif) => self.app.handle_async(notif),
                Err(_) => self.fail(&format!(
                    "timed out waiting for {} pending svn operation(s)",
                    self.app.pending
                )),
            }
        }
    }

    /// Render once into an off-screen terminal (catches draw-time panics).
    fn draw_once(&mut self) {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|f| {
                let _ = self.app.draw(f);
            })
            .expect("draw");
    }

    /// Switch tabs via the app's own key bindings ('1'/'2'/'3').
    fn goto_tab(&mut self, tab_key: char) {
        self.input(key(KeyCode::Char(tab_key)));
    }

    /// A random ASCII-alphanumeric word (length 4..=16) from `text`.
    fn pick_word(&mut self, text: &str) -> Option<String> {
        let words: Vec<&str> = text
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|w| (4..=16).contains(&w.len()))
            .collect();
        if words.is_empty() {
            None
        } else {
            Some(words[self.rng.below(words.len())].to_string())
        }
    }

    /// Filter the status tree to `path` and put the selection on it.
    fn filter_select(&mut self, path: &str) {
        self.clear_tree_filter();
        self.input(key(KeyCode::Char('/')));
        self.type_str(path);
        self.input(key(KeyCode::Enter));
        self.tree_filter = path.to_string();
        let mut steps = self.app.status.tree.visible_len();
        while self.app.status.tree.selection_path().as_deref() != Some(path) {
            if steps == 0 {
                self.fail(&format!("file {path} not selectable in the status tree"));
            }
            steps -= 1;
            self.input(key(KeyCode::Char('j')));
        }
    }

    fn clear_tree_filter(&mut self) {
        if self.tree_filter.is_empty() {
            return;
        }
        self.input(key(KeyCode::Char('/')));
        for _ in 0..self.tree_filter.chars().count() {
            self.input(key(KeyCode::Backspace));
        }
        self.input(key(KeyCode::Enter));
        self.tree_filter.clear();
    }

    /// Invariant: the working copy is clean. A dirty wc here means an
    /// earlier commit/revert went sideways — clean up externally and count
    /// the anomaly so later rounds stay independent.
    fn ensure_clean(&mut self) {
        let out = svnui::test_support::svn(&self.wc, &["status"]);
        if !out.trim().is_empty() {
            eprintln!(
                "stress: wc not clean in round {}, hard-reverting:\n{out}",
                self.round
            );
            self.stats.anomalies += 1;
            svnui::test_support::svn(&self.wc, &["revert", "-R", "."]);
        }
    }

    /// Per-round invariants: no unexpected error popups, no fatal error,
    /// no pending operations, no leaked popups.
    fn check_invariants(&mut self) {
        let errors: Vec<String> = self
            .app
            .popups
            .iter()
            .filter_map(|p| match p {
                Popup::Msg(m) if m.is_error => Some(m.message.clone()),
                _ => None,
            })
            .collect();
        for e in errors {
            if is_benign_error(&e) {
                eprintln!(
                    "stress: tolerated benign error popup (round {}): {e}",
                    self.round
                );
                self.stats.benign_errors += 1;
            } else {
                self.fail(&format!("unexpected error popup: {e}"));
            }
        }
        // drop tolerated error popups so the stack is clean
        self.app
            .popups
            .retain(|p| !matches!(p, Popup::Msg(m) if m.is_error));
        if self.app.pending != 0 {
            self.fail(&format!("pending={} after settle", self.app.pending));
        }
        if let Some(f) = self.app.fatal_error.clone() {
            self.fail(&format!("fatal error: {f}"));
        }
        if !self.app.popups.is_empty() {
            self.fail(&format!("{} popup(s) left open", self.app.popups.len()));
        }
    }

    // ----- round actions -----

    /// Log tab: scroll to a random loaded revision, open its commit info
    /// popup (`v`), close it.
    fn act_commit_info(&mut self) {
        self.action = "log/commit-info";
        self.goto_tab('2');
        let len = self.app.log.entries.len();
        if len == 0 {
            return;
        }
        self.input(key(KeyCode::Home));
        for _ in 0..self.rng.below(len.min(60)) {
            self.input(key(KeyCode::Char('j')));
        }
        self.settle(); // scrolling near the bottom pages in older revisions
        self.input(key(KeyCode::Char('v')));
        if !matches!(self.app.popups.last(), Some(Popup::Output(_))) {
            self.fail("expected commit info popup");
        }
        self.draw_once();
        self.input(key(KeyCode::Esc));
    }

    /// Log tab: open a revision diff (`d`), search inside the diff
    /// (`/` + keyword from the diff content, Enter, `n`), close it.
    fn act_revision_diff(&mut self) {
        self.action = "log/revision-diff";
        self.goto_tab('2');
        let len = self.app.log.entries.len();
        if len == 0 {
            return;
        }
        self.input(key(KeyCode::Home));
        for _ in 0..self.rng.below(len.min(60)) {
            self.input(key(KeyCode::Char('j')));
        }
        self.settle();
        let rev = self.app.log.selection_revision().unwrap_or(0);
        self.input(key(KeyCode::Char('d')));
        self.settle();
        if !matches!(self.app.popups.last(), Some(Popup::Diff(_))) {
            self.fail(&format!("expected fullscreen diff popup for r{rev}"));
        }
        self.stats.diffs += 1;
        self.draw_once();
        let text = match self.app.popups.last() {
            Some(Popup::Diff(d)) => d
                .view
                .parsed
                .lines
                .iter()
                .map(|l| l.content.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if let Some(w) = self.pick_word(&text) {
            self.input(key(KeyCode::Char('/')));
            self.type_str(&w);
            self.input(key(KeyCode::Enter));
            for _ in 0..3 {
                self.input(key(KeyCode::Char('n')));
            }
            self.stats.searches += 1;
            self.draw_once();
            self.input(key(KeyCode::Esc)); // clear search highlights
        }
        self.input(key(KeyCode::Esc)); // close the popup
        if !self.app.popups.is_empty() {
            self.fail("diff popup did not close");
        }
    }

    /// File finder (Ctrl+P): type a query, Ctrl+B blame the highlighted
    /// file, search inside the blame, close everything.
    fn act_finder_blame(&mut self) {
        self.action = "finder/blame";
        self.input(ctrl('p'));
        self.settle();
        if !matches!(self.app.popups.last(), Some(Popup::FileFinder(_))) {
            self.fail("expected file finder popup");
        }
        // query: a fragment of a random versioned file's name (always a
        // subsequence of that file, so at least one result exists)
        let f = self.files[self.rng.below(self.files.len())].clone();
        let name = f.rsplit('/').next().unwrap_or(&f);
        let stem: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(8)
            .collect();
        if !stem.is_empty() {
            self.type_str(&stem);
        }
        self.draw_once();
        self.input(ctrl('b'));
        self.settle();
        enum BlameState {
            Open,
            Benign(String),
            Bad,
        }
        let state = match self.app.popups.last() {
            Some(Popup::Blame(_)) => BlameState::Open,
            Some(Popup::Msg(m)) if m.is_error && is_benign_error(&m.message) => {
                BlameState::Benign(m.message.clone())
            }
            _ => BlameState::Bad,
        };
        match state {
            BlameState::Open => {}
            BlameState::Benign(msg) => {
                eprintln!(
                    "stress: benign blame error on {f} (round {}): {msg}",
                    self.round
                );
                self.stats.benign_errors += 1;
                self.input(key(KeyCode::Esc)); // close error popup
                self.input(key(KeyCode::Esc)); // close finder
                return;
            }
            BlameState::Bad => self.fail(&format!("expected blame popup for {f}")),
        }
        self.stats.blames += 1;
        self.draw_once();
        let text = match self.app.popups.last() {
            Some(Popup::Blame(b)) => b
                .lines
                .iter()
                .map(|l| l.content.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        if let Some(w) = self.pick_word(&text) {
            self.input(key(KeyCode::Char('/')));
            self.type_str(&w);
            self.input(key(KeyCode::Enter));
            for _ in 0..2 {
                self.input(key(KeyCode::Char('n')));
            }
            self.stats.searches += 1;
            self.input(key(KeyCode::Esc)); // clear search highlights
        }
        self.input(key(KeyCode::Esc)); // close blame
        self.input(key(KeyCode::Esc)); // close finder
        if !self.app.popups.is_empty() {
            self.fail("finder/blame popups did not close");
        }
    }

    /// Log tab: `/` + keyword + Enter searches the full history
    /// (`svn log --search`), Esc returns to the normal log view.
    fn act_log_search(&mut self) {
        self.action = "log/search";
        self.goto_tab('2');
        let len = self.app.log.entries.len();
        if len == 0 {
            return;
        }
        let mut candidates: Vec<String> = Vec::new();
        for _ in 0..8 {
            let e = &self.app.log.entries[self.rng.below(len)];
            candidates.push(e.author.clone());
            candidates.push(e.message.clone());
        }
        let mut word = None;
        for c in &candidates {
            if let Some(w) = self.pick_word(c) {
                word = Some(w);
                break;
            }
        }
        let word = word.unwrap_or_else(|| "stress".to_string());
        self.input(key(KeyCode::Char('/')));
        if !matches!(self.app.popups.last(), Some(Popup::LogSearch(_))) {
            self.fail("expected log search popup");
        }
        self.type_str(&word);
        self.input(key(KeyCode::Enter));
        self.settle(); // svn log --search scans the full history
        self.stats.searches += 1;
        self.draw_once();
        self.input(key(KeyCode::Esc)); // clear filter + reload the log
        self.settle();
    }

    /// Status tab: modify 1-3 random tracked files, refresh (F5), view one
    /// file diff, then either commit (A + c + message + Enter + y) or
    /// revert each file (filter + r + y).
    fn act_modify(&mut self) {
        self.action = "status/modify";
        self.goto_tab('1');
        self.ensure_clean();
        let n = 1 + self.rng.below(3);
        let mut picked: Vec<String> = Vec::new();
        while picked.len() < n {
            let f = self.files[self.rng.below(self.files.len())].clone();
            if !picked.contains(&f) {
                picked.push(f);
            }
        }
        let tag = format!("svnui stress round {} {:x}", self.round, self.rng.next());
        for f in &picked {
            append_line(&self.wc.join(f), &format!("// {tag}"));
        }
        self.input(key(KeyCode::F(5)));
        self.settle();
        // view one file's diff fullscreen
        let f0 = picked[0].clone();
        self.filter_select(&f0);
        self.input(key(KeyCode::Char('d')));
        self.settle();
        if !matches!(self.app.popups.last(), Some(Popup::Diff(_))) {
            self.fail(&format!("expected file diff popup for {f0}"));
        }
        self.stats.diffs += 1;
        self.draw_once();
        self.input(key(KeyCode::Esc));
        if self.rng.below(2) == 0 {
            self.commit_picked();
        } else {
            self.revert_picked(&picked);
        }
    }

    /// Commit everything currently changed (the files this round modified):
    /// stage all, type a generated message, confirm.
    fn commit_picked(&mut self) {
        self.action = "status/commit";
        self.input(key(KeyCode::Char('A')));
        self.input(key(KeyCode::Char('c')));
        let msg = format!("stress round {} {:x}", self.round, self.rng.next());
        self.type_str(&msg);
        self.input(key(KeyCode::Enter));
        if !matches!(self.app.popups.last(), Some(Popup::Confirm(_))) {
            self.fail("expected commit confirmation popup");
        }
        self.draw_once();
        self.input(key(KeyCode::Char('y')));
        self.settle();
        if !matches!(self.app.popups.last(), Some(Popup::Output(_))) {
            self.fail("expected commit output popup");
        }
        self.draw_once();
        self.input(key(KeyCode::Esc));
        self.clear_tree_filter();
        self.stats.commits += 1;
    }

    /// Revert each modified file via the app's revert confirmation flow.
    fn revert_picked(&mut self, picked: &[String]) {
        self.action = "status/revert";
        for f in picked {
            let f = f.clone();
            self.filter_select(&f);
            self.input(key(KeyCode::Char('r')));
            if !matches!(self.app.popups.last(), Some(Popup::Confirm(_))) {
                self.fail(&format!("expected revert confirmation for {f}"));
            }
            self.input(key(KeyCode::Char('y')));
            self.settle();
            // dismiss the "reverted" info popup
            if matches!(self.app.popups.last(), Some(Popup::Msg(_))) {
                self.input(key(KeyCode::Esc));
            }
            self.stats.reverts += 1;
        }
        self.clear_tree_filter();
        self.ensure_clean();
    }

    /// Save the working-copy changes as a patch (`P`), preview it in the
    /// patches tab, delete it (d + y), then revert the change.
    fn act_patch(&mut self) {
        self.action = "patch";
        self.goto_tab('1');
        self.ensure_clean();
        let f = self.files[self.rng.below(self.files.len())].clone();
        let tag = format!(
            "svnui stress patch round {} {:x}",
            self.round,
            self.rng.next()
        );
        append_line(&self.wc.join(&f), &format!("// {tag}"));
        self.input(key(KeyCode::F(5)));
        self.settle();
        self.input(key(KeyCode::Char('P')));
        self.settle();
        if !matches!(self.app.popups.last(), Some(Popup::Msg(m)) if !m.is_error) {
            self.fail("expected 'patch saved' info popup");
        }
        self.input(key(KeyCode::Esc));
        self.goto_tab('3');
        if self.app.patches.entries.is_empty() {
            self.fail("saved patch not listed in the patches tab");
        }
        self.input(key(KeyCode::Home));
        self.input(key(KeyCode::Enter)); // preview
        if !matches!(self.app.popups.last(), Some(Popup::Diff(_))) {
            self.fail("expected patch preview popup");
        }
        self.draw_once();
        self.input(key(KeyCode::Esc));
        self.input(key(KeyCode::Char('d'))); // delete
        if !matches!(self.app.popups.last(), Some(Popup::Confirm(_))) {
            self.fail("expected patch delete confirmation");
        }
        self.input(key(KeyCode::Char('y')));
        if !matches!(self.app.popups.last(), Some(Popup::Msg(m)) if !m.is_error) {
            self.fail("expected 'patch deleted' info popup");
        }
        self.input(key(KeyCode::Esc));
        self.goto_tab('1');
        // revert the change so the wc is clean again
        self.revert_picked(&[f]);
        self.stats.patches += 1;
    }

    /// F5 refresh; occasionally a full `svn update` (u + y).
    fn act_refresh(&mut self) {
        self.action = "refresh";
        self.goto_tab('1');
        self.input(key(KeyCode::F(5)));
        self.settle();
        self.stats.refreshes += 1;
        if self.rng.below(3) == 0 {
            self.input(key(KeyCode::Char('u')));
            if !matches!(self.app.popups.last(), Some(Popup::Confirm(_))) {
                self.fail("expected update confirmation popup");
            }
            self.input(key(KeyCode::Char('y')));
            self.settle();
            if !matches!(self.app.popups.last(), Some(Popup::Output(_))) {
                self.fail("expected update output popup");
            }
            self.draw_once();
            self.input(key(KeyCode::Esc));
            self.stats.updates += 1;
        }
    }
}

#[test]
fn stress_workflow() {
    if std::env::var("SVNUI_STRESS").as_deref() != Ok("1") {
        eprintln!("stress test skipped (set SVNUI_STRESS=1 and SVNUI_STRESS_WC to run it)");
        return;
    }
    let wc = PathBuf::from(
        std::env::var("SVNUI_STRESS_WC").expect("SVNUI_STRESS_WC must be set when SVNUI_STRESS=1"),
    );
    assert!(
        wc.join(".svn").is_dir(),
        "SVNUI_STRESS_WC={} is not an svn working copy",
        wc.display()
    );
    let rounds: usize = std::env::var("SVNUI_STRESS_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let mut seed: u64 = std::env::var("SVNUI_STRESS_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x5EED_0001);
    if seed == 0 {
        seed = 0x9E37_79B9_7F4A_7C15; // xorshift degenerates on a zero state
    }

    let files = list_wc_files(&wc);
    assert!(!files.is_empty(), "no versioned text files in the wc");
    println!(
        "stress config: wc={} files={} rounds={rounds} seed={seed}",
        wc.display(),
        files.len()
    );

    let (tx, rx) = unbounded();
    let queue = Queue::new();
    let ctx = Context {
        queue: queue.clone(),
        theme: Theme::default(),
    };
    let svn = Svn::new(wc.clone(), tx);
    let mut app = App::new(wc.clone(), svn, ctx);
    // keep patch files out of the user's data dir and out of the wc
    let patch_dir = wc.parent().unwrap_or(wc.as_path()).join("patches");
    app.patches.set_dir(patch_dir);
    app.start();

    let mut h = Harness {
        app,
        rx,
        wc,
        files,
        rng: XorShift(seed),
        seed,
        round: 0,
        action: "startup",
        stats: Stats::default(),
        tree_filter: String::new(),
    };
    h.settle();
    if h.app.fatal_error.is_some() || h.app.svn_info.is_none() {
        h.fail("startup failed (svn info)");
    }
    if h.app.log.entries.is_empty() {
        h.fail("log is empty after startup");
    }
    println!(
        "stress startup ok: branch={} head=r{} loaded_log={}",
        h.app.svn_info.as_ref().unwrap().branch_label(),
        h.app.svn_info.as_ref().unwrap().revision,
        h.app.log.entries.len()
    );

    let started = Instant::now();
    for round in 1..=rounds {
        h.round = round;
        match h.rng.below(100) {
            0..=14 => h.act_commit_info(),
            15..=29 => h.act_revision_diff(),
            30..=44 => h.act_finder_blame(),
            45..=59 => h.act_log_search(),
            60..=79 => h.act_modify(),
            80..=94 => h.act_patch(),
            _ => h.act_refresh(),
        }
        h.app.tick(); // spinner tick, as in main.rs
        h.settle();
        h.check_invariants();
        if round % 25 == 0 {
            println!(
                "stress: round {round}/{rounds} ok ({}) [{:.0?} elapsed]",
                h.stats,
                started.elapsed()
            );
        }
    }

    println!(
        "stress done: {rounds} rounds, seed={seed}, {} [{:.0?}]",
        h.stats,
        started.elapsed()
    );
}
