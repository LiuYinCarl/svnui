#!/bin/bash
# Set up the demo environment used by record_demos.sh:
#   $SVNUI_DEMO_DIR/
#     repo/    hotcopy of the source SVN repo (disposable; the commit demo
#              really commits into it)
#     wc/      checkout of trunk + 4 handcrafted local changes
#     patches/ empty patch dir (SVNUI_PATCH_DIR during recording)
#     wc-base/ pristine snapshot of wc; record_demos.sh resets wc from it
#              before every recording
#
# The source repo defaults to a local git2svn conversion of spdlog; override
# with SVNUI_DEMO_SRC_REPO=<path-to-svn-repo>. Any reasonably large public
# repo works, but the committed .demo scripts reference spdlog paths
# (pattern_formatter etc.), so with a different source you must adapt
# scripts/demos/*.demo.
set -euo pipefail

DEMO_DIR="${SVNUI_DEMO_DIR:-/tmp/svnui-demo}"
SRC_REPO="${SVNUI_DEMO_SRC_REPO:-$HOME/dev/spdlog-svn-repo}"

if [ ! -d "$SRC_REPO/db" ]; then
    echo "error: source SVN repo not found at $SRC_REPO" >&2
    echo "       set SVNUI_DEMO_SRC_REPO, e.g. after converting a public git repo" >&2
    echo "       with git2svn (see agent.md '压力测试' for git2svn usage)" >&2
    exit 1
fi

echo "== setting up demo env in $DEMO_DIR (source: $SRC_REPO)"
rm -rf "$DEMO_DIR"
mkdir -p "$DEMO_DIR/patches"

svnadmin hotcopy "$SRC_REPO" "$DEMO_DIR/repo" > /dev/null
svn co -q "file://$DEMO_DIR/repo/trunk" "$DEMO_DIR/wc"
cd "$DEMO_DIR/wc"

# Handcrafted local changes for the status tab: one modified file at the top
# level of example/, one deleted + one modified file under include/, one
# unversioned file. Staging/commit/revert/apply demos all operate on these.
python3 - <<'EOF'
import pathlib
p = pathlib.Path("include/spdlog/spdlog.h")
p.write_text(p.read_text() + "\n// demo: note about async flushing\n")
c = pathlib.Path("example/example.cpp")
c.write_text(c.read_text().replace("int main", "// demo tweak\nint main", 1))
EOF
printf 'release checklist:\n- bump version\n- update changelog\n' > notes.txt
svn rm -q include/spdlog/fmt/fmt.h

svn status
rsync -a "$DEMO_DIR/wc/" "$DEMO_DIR/wc-base/"
echo "== demo env ready"
