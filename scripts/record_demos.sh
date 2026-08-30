#!/bin/bash
# Record all demo scripts (scripts/demos/*.demo) into docs/screenshots/.
#
# Requires: asciinema, agg, python3 + Pillow, a built target/release/svnui.
# The demo environment (SVNUI_DEMO_DIR, default /tmp/svnui-demo) is created
# automatically by setup_demo_env.sh on first run.
#
# Per demo NAME.demo the pipeline is:
#   record_demo.py drives svnui in a pty -> asciinema .cast
#   -> agg renders docs/screenshots/NAME.gif
#   -> for every `snap S` line in the .demo script, agg re-renders that
#      timestamp into docs/screenshots/S.png (deterministic stills)
#
# Useful overrides: SVNUI_DEMO_DIR, SVNUI_DEMO_SRC_REPO, SVNUI_DEMO_SIZE,
# AGG_THEME, AGG_SPEED.
set -euo pipefail
cd "$(dirname "$0")/.."

DEMO_DIR="${SVNUI_DEMO_DIR:-/tmp/svnui-demo}"
IMG=docs/screenshots
SIZE="${SVNUI_DEMO_SIZE:-110x32}"
THEME="${AGG_THEME:-monokai}"
SPEED="${AGG_SPEED:-1.3}"
ORDER="help info status blame commit log patches"

for tool in asciinema agg; do
    command -v "$tool" >/dev/null || { echo "error: $tool not installed" >&2; exit 1; }
done
python3 -c "import PIL" 2>/dev/null || { echo "error: python3 Pillow not installed" >&2; exit 1; }
[ -x target/release/svnui ] || { echo "error: run cargo build --release first" >&2; exit 1; }

if [ ! -d "$DEMO_DIR/wc-base" ]; then
    scripts/setup_demo_env.sh
fi

mkdir -p "$DEMO_DIR/out" "$IMG"
export SVNUI_PATCH_DIR="$DEMO_DIR/patches"

for name in $ORDER; do
    echo "== recording $name"
    [ "$name" = patches ] && rm -f "$DEMO_DIR/patches"/*.patch 2>/dev/null
    rsync -a --delete "$DEMO_DIR/wc-base/" "$DEMO_DIR/wc/"
    asciinema rec --headless --overwrite --quiet --window-size "$SIZE" \
        --idle-time-limit 2 \
        --command "python3 scripts/record_demo.py scripts/demos/$name.demo --cwd $DEMO_DIR/wc --snaps $DEMO_DIR/out/$name.snaps.json" \
        "$DEMO_DIR/out/$name.cast"
    agg --theme "$THEME" --speed "$SPEED" --last-frame-duration 2.5 \
        "$DEMO_DIR/out/$name.cast" "$IMG/$name.gif" >/dev/null
    # deterministic stills from `snap` points
    python3 - "$DEMO_DIR/out/$name" "$THEME" <<'PY'
import json, subprocess, sys, time
from PIL import Image

base, theme = sys.argv[1], sys.argv[2]
try:
    snaps = json.load(open(f"{base}.snaps.json"))
except FileNotFoundError:
    snaps = {}
for snap_name, t in snaps.items():
    tmp = f"{base}-snap-{snap_name}.gif"
    # --idle-time-limit huge: snap times are raw script times, but the main
    # render caps idle time, which shifts agg's timeline
    cmd = ["agg", "--theme", theme, "--idle-time-limit", "1000000",
           "--select", f"{t}s", f"{base}.cast", tmp]
    for attempt in range(3):  # agg occasionally flakes; it is idempotent
        r = subprocess.run(cmd, capture_output=True)
        if r.returncode == 0:
            break
        if attempt == 2:
            sys.exit(f"agg failed for snap {snap_name}: {r.stderr.decode()}")
        time.sleep(1)
    im = Image.open(tmp)
    im.seek(0)
    im.convert("RGB").save(f"docs/screenshots/{snap_name}.png")
    print(f"  snap {snap_name} @ {t:.1f}s -> docs/screenshots/{snap_name}.png")
PY
done
echo "done -> $IMG"
