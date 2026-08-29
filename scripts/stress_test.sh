#!/usr/bin/env bash
# Stress-test harness for svnui: convert a real git repo into an SVN repo
# (via git2svn), check out a working copy, and drive the app headlessly
# against it (tests/stress.rs).
#
# Usage:
#   scripts/stress_test.sh
#
# Env knobs:
#   STRESS_GIT_REPO     git repo to convert   (default ~/dev/github/openless)
#   STRESS_GIT_BRANCH   branch to convert     (default: current branch,
#                       falling back to main/master when detached)
#   SVNUI_STRESS_ROUNDS stress rounds         (default 200)
#   SVNUI_STRESS_SEED   PRNG seed             (default fixed)
#   GIT2SVN_DIR         git2svn checkout      (default ~/dev/github/git2svn)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GIT2SVN_DIR="${GIT2SVN_DIR:-$HOME/dev/github/git2svn}"
GIT2SVN_BIN="$GIT2SVN_DIR/git2svn"
STRESS_GIT_REPO="${STRESS_GIT_REPO:-$HOME/dev/github/openless}"
STRESS_GIT_BRANCH="${STRESS_GIT_BRANCH:-}"
export SVNUI_STRESS_ROUNDS="${SVNUI_STRESS_ROUNDS:-200}"
export SVNUI_STRESS_SEED="${SVNUI_STRESS_SEED:-20260829}"

STRESS_DIR="$ROOT/target/tmp/stress"
SVN_REPO="$STRESS_DIR/svn-repo"
WC="$STRESS_DIR/wc"

# friendly preflight: fail fast with actionable messages instead of
# cryptic git2svn/go-build errors when the defaults don't apply
if ! git -C "$STRESS_GIT_REPO" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    echo "error: STRESS_GIT_REPO ($STRESS_GIT_REPO) is not a git repo" >&2
    echo "       point it at a git repo to convert, e.g." >&2
    echo "       STRESS_GIT_REPO=/path/to/repo scripts/stress_test.sh" >&2
    exit 1
fi
if [[ ! -d "$GIT2SVN_DIR" ]]; then
    echo "error: GIT2SVN_DIR ($GIT2SVN_DIR) does not exist" >&2
    echo "       git clone https://github.com/LiuYinCarl/git2svn" >&2
    echo "       or set GIT2SVN_DIR to an existing git2svn checkout" >&2
    exit 1
fi

echo "== svnui stress test =="
echo "   git repo : $STRESS_GIT_REPO"
echo "   rounds   : $SVNUI_STRESS_ROUNDS   seed: $SVNUI_STRESS_SEED"

# 1. git2svn binary (build once, reuse)
if [[ ! -x "$GIT2SVN_BIN" ]]; then
    echo ">> building git2svn in $GIT2SVN_DIR"
    (cd "$GIT2SVN_DIR" && go build -o git2svn .)
fi

# 2. branch to convert
if [[ -z "$STRESS_GIT_BRANCH" ]]; then
    STRESS_GIT_BRANCH="$(git -C "$STRESS_GIT_REPO" rev-parse --abbrev-ref HEAD)"
    if [[ "$STRESS_GIT_BRANCH" == "HEAD" ]]; then # detached HEAD
        for cand in main master; do
            if git -C "$STRESS_GIT_REPO" rev-parse --verify --quiet "$cand" >/dev/null; then
                STRESS_GIT_BRANCH="$cand"
                break
            fi
        done
    fi
fi
echo "   branch   : $STRESS_GIT_BRANCH ($(git -C "$STRESS_GIT_REPO" rev-list --count --first-parent "$STRESS_GIT_BRANCH") first-parent commits)"

# 3. convert git -> svn (wipe and recreate)
rm -rf "$STRESS_DIR"
mkdir -p "$STRESS_DIR"
echo ">> converting to $SVN_REPO (this takes a minute or two)"
"$GIT2SVN_BIN" dump "$STRESS_GIT_REPO" "$SVN_REPO" "$STRESS_GIT_BRANCH" "$STRESS_DIR/git2svn-map.txt"

# 4. check out the working copy
echo ">> checking out $WC"
svn checkout -q "file://$SVN_REPO/trunk" "$WC"

# 5. run the headless stress test
echo ">> running stress test"
start=$SECONDS
SVNUI_STRESS=1 SVNUI_STRESS_WC="$WC" \
    cargo test --manifest-path "$ROOT/Cargo.toml" --test stress -- --nocapture --test-threads=1

echo "== stress test OK in $((SECONDS - start))s =="
echo "   svn repo : $SVN_REPO"
echo "   wc       : $WC"
echo "   patches  : $STRESS_DIR/patches"
echo "   map file : $STRESS_DIR/git2svn-map.txt"
