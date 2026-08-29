# svnui

[中文文档](README.md) | English

An SVN (Subversion) terminal UI client inspired by [gitui](https://github.com/gitui-org/gitui), written in Rust with [ratatui](https://github.com/ratatui/ratatui).

![svnui demo](https://github.com/LiuYinCarl/svnui/releases/download/assets/svnui.gif)

## Features

Covers the most common daily SVN operations:

| Feature | Description |
| --- | --- |
| **Status view** | `svn status` rendered as a collapsible directory tree, with color-coded M/A/D/C/? states |
| **Branch info** | The status bar always shows the current branch; the commit confirmation popup shows the target branch and the files to be committed |
| **Diff pane** | Selecting a file automatically shows its `svn diff` with line numbers and +/− highlighting; unversioned files show their content directly |
| **Staging / commit** | `space` stages (adds to the commit set), `A` stages all, `U` unstages all; staging an unversioned file runs `svn add` automatically; committing with an empty set is rejected. The input box supports CJK/wide chars and multi-line paste; `Tab` recalls recent commit messages |
| **Log view** | `svn log -v` revision list with changed paths and message details; scrolling to the bottom loads older revisions (paged, 50 at a time); `/` opens a search box (live-filters the loaded list while typing, Enter searches the full history with `svn log --search`, also paged); mark revisions with `space` and view a combined diff with `d`/`Enter` |
| **File history** | `t` shows the selected file's `svn log` history; `Enter` in the popup opens that revision's diff, `b` opens its blame |
| **File finder** | `Ctrl+p` fzf-style fuzzy file search with highlighted matches; Enter jumps to the file's history, `Ctrl+b` opens its blame |
| **Blame** | `svn blame` with per-revision coloring (status tree `b`, file-history popup `b`, file finder `Ctrl+b`); in the popup `j/k` moves the cursor line, `Enter` opens the diff of that line's revision |
| **Revert** | `svn revert` (with confirmation) |
| **Update** | `svn update` and update-to-revision (`svn update -r N`), both confirmed; the confirmation shows the working-copy path being updated |
| **Conflict resolution** | `svn resolve --accept=working` (with confirmation) |
| **Patch management** | `P` saves working-copy changes as a timestamped patch file (an `svn diff` snapshot; the working copy is not reverted); `3` opens the patches tab (newest first) where `Enter`/`p` previews (reusing the diff popup), `a` applies (`svn patch`, confirmed) and `d` deletes (confirmed). Patches live in the platform data dir; override with `SVNUI_PATCH_DIR` |
| **Filtering** | `/` filters files by path (status tab, popup input, live filtering; Esc clears an active filter) / searches commits (log tab) |
| **Repo info** | `i` opens a repository overview popup: working-copy info (path/URL/branch/revision/last change), remote HEAD comparison (how many revisions behind, last commit), and a change summary (per-status counts + commit-set size), all color-coded |
| **Help** | `?` shows all key bindings |
| **Async execution** | Every svn command runs on a background thread; the UI never blocks and shows a spinner |

## Install & run

```bash
# requires the Rust toolchain and the svn client
cargo build --release

# run inside an SVN working copy
svnui
# or point at a directory
svnui /path/to/working-copy
```

Prebuilt binaries for Linux (x86_64), macOS (arm64) and Windows (x86_64) are attached to every [GitHub Release](https://github.com/LiuYinCarl/svnui/releases).

## Key bindings

| Key | Action |
| --- | --- |
| `q` | Quit |
| `j` / `↓` / `k` / `↑` | Move selection |
| `h` / `←` / `l` / `→` | Collapse / expand directory |
| `g` / `G` | Jump to first / last entry |
| `PgUp` / `PgDn` | Page up / down |
| `space` | Stage / unstage (toggle commit set) |
| `A` / `U` | Stage all / unstage all |
| `a` | `svn add` selected unversioned files |
| `r` | `svn revert` selected files (confirmed) |
| `x` | Resolve conflict (accept working copy) |
| `c` | Focus the commit message input |
| `Enter` | Commit (inside the input) |
| `Tab` | Commit input: list recent messages, select to fill |
| `u` | `svn update` |
| `d` | Fullscreen diff |
| `b` | Blame file (status tab / file-history popup) |
| `t` | Selected file's commit history |
| `Ctrl+p` | Fuzzy file finder (Enter opens file history) |
| `Ctrl+b` | File finder: blame the highlighted file |
| `/` | Filter files (status tab, popup input, live) / search commits (log tab, popup input, Enter searches full history) / incremental text search inside diff & blame popups (live highlight, scrolls to match) |
| `n` / `N` | Diff / blame popup search: next / previous match (wraps) |
| `Enter` | Blame popup: diff of the cursor line's revision |
| `h` / `l` | Diff / blame views: scroll long lines left / right (narrow terminals) |
| `i` | Repository overview (local info + remote HEAD comparison + change summary) |
| `F5` / `R` | Refresh status / log / patch list |
| `P` | Save working-copy changes as a patch file (snapshot, no revert) |
| `Tab` / `Shift+Tab` | Cycle pane focus / switch tabs |
| `1` / `2` / `3` | Status / log / patches tabs |
| `Enter` / `d` | Log tab: diff of the selected (or marked) revisions |
| `space` | Log tab: mark / unmark revision |
| `o` | Log tab: update working copy to the selected revision |
| `v` | Log tab: full commit info |
| `Enter` / `p` | Patches tab: preview patch (diff view) |
| `a` | Patches tab: apply patch (`svn patch`, confirmed) |
| `d` | Patches tab: delete patch file (confirmed) |
| `?` | Help |
| `Esc` | Close popup / cancel / clear the status-tab file filter (while searching: first cancels input or clears highlights, a second press closes the popup) |

## CI/CD

[![CI](https://github.com/LiuYinCarl/svnui/actions/workflows/ci.yml/badge.svg)](https://github.com/LiuYinCarl/svnui/actions/workflows/ci.yml)
[![Release](https://github.com/LiuYinCarl/svnui/actions/workflows/release.yml/badge.svg)](https://github.com/LiuYinCarl/svnui/actions/workflows/release.yml)

`.github/workflows/` contains three pipelines:

- **ci.yml** — runs on push / PR: fmt, clippy (zero-warning gate), full tests on Linux/macOS, coverage gate (≥ 80%), release builds on three platforms, and headless stress tests: a parallel matrix of 13 popular open-source repos across languages — redis / tmux (C), clap (Rust), slugify (JS), ts-node (TS), requests (Python), gin (Go), nlohmann/json (C++), gson (Java), jekyll (Ruby), composer (PHP), elixir (Elixir), ohmyzsh (Shell) — each shallow-cloned at depth 500, converted to SVN with git2svn, and driven through 60 randomized rounds (15-minute cap per leg); the source commit is logged so failures can be reproduced.
- **bump.yml** — runs on every push to master/main: bumps the patch version in `Cargo.toml`, commits it (message carries `[skip ci]` so it doesn't trigger itself), tags `vX.Y.Z`, pushes, then calls release.yml to publish. Multiple pushes to the same branch are serialized; a queued run first syncs the branch tip so it never computes a stale version.
- **release.yml** — runs on `v*` tags (also called by bump.yml): verifies the tag matches the `Cargo.toml` version, builds release binaries on Linux (x86_64), macOS (arm64) and Windows (x86_64), and creates a GitHub Release.

### Releasing

Day to day, just push to master/main: bump.yml bumps the patch version and releases automatically — no manual steps.

For a minor/major release, set `version` in `Cargo.toml` to the target before pushing (bump only increments the patch part on top of it). The manual tag flow still works too (the tag `vX.Y.Z` must match the `Cargo.toml` version):

```bash
git tag v0.2.0
git push origin v0.2.0
```

## Performance (very large SVN projects)

Optimized for working copies with 100k+ files:

- O(n) tree construction (single HashMap assembly pass)
- Virtualized tree / diff / blame rendering: each draw only processes the visible window
- Cached per-directory staged counts, zero recomputation while navigating
- `cargo bench --bench tree` (criterion benchmarks) + `cargo test perf` (CI time gates against complexity regressions)

## Design notes

The architecture follows gitui:

- `src/main.rs` — terminal setup, event loop (`crossbeam_channel::select` multiplexing input / async svn results / spinner ticks)
- `src/app.rs` — app state, tabs, popup stack, async operation dispatch (gitui's `App`/`Gitui`)
- `src/queue.rs` — inter-component event queue (gitui's `Queue` + `NeedsUpdate`)
- `src/svn/` — svn CLI wrapper and output parsers (gitui's `asyncgit`): all operations run on threads and report back over channels
- `src/components/` — file tree, diff, log, blame, commit input, help, etc. (gitui's `components/`)
- `src/popups/` — confirm, message, output viewer, fullscreen diff, etc. (gitui's `popups/`)
- `src/keys.rs` — central key-binding definitions (gitui's `keys/`)

SVN has no staging area like git, so "staging" is implemented as a **commit set**: the files marked to go into the next commit; staging an unversioned file runs `svn add` automatically; committing with an empty set is rejected.

## Testing

```bash
cargo test                 # unit/integration tests (some create real temporary SVN repos)
cargo llvm-cov             # coverage report (needs cargo-llvm-cov + llvm-tools-preview)
cargo clippy --all-targets # zero warnings
```

Test strategy:

- **Parsers / models**: unit tests against sample svn output
- **UI components**: off-screen rendering with `ratatui::TestBackend` driven by synthetic crossterm events
- **svn command layer**: tests `svnadmin create` a temporary repo and run real status/diff/log/blame/add/revert/commit/update/resolve
- **App state machine**: feeds `AsyncSvnNotification` and `InternalEvent` directly to cover every branch, including error paths
- **Event loop**: `run()` is generic and driven to exit via TestBackend

The full workflow is verified on macOS (svn 1.14.5).
