#!/usr/bin/env python3
"""Drive svnui in a pty from a script file, forwarding output to stdout.

Intended to run under `asciinema rec --headless --window-size COLSxROWS
--command "python3 scripts/record_demo.py <script> --cwd <wc>"`.

Script format (one command per line, '#' starts a comment):
    wait <seconds>          pause
    type <text>             type text (per-char delay for a natural look)
    key <name>              enter esc tab backspace space up down left right
                            home end f5
    ctrl <letter>           Ctrl+<letter>
    snap <name>             mark the current timestamp as a screenshot point;
                            written to the --snaps JSON file for still extraction

The app should quit itself (script ends with `type q`). As a safety net the
driver sends Esc then q after the settle time, then SIGTERM.
"""

import argparse
import fcntl
import os
import select
import signal
import struct
import sys
import termios
import time

KEYS = {
    "enter": b"\r",
    "esc": b"\x1b",
    "tab": b"\t",
    "backspace": b"\x7f",
    "space": b" ",
    "up": b"\x1b[A",
    "down": b"\x1b[B",
    "right": b"\x1b[C",
    "left": b"\x1b[D",
    "home": b"\x1b[H",
    "end": b"\x1b[F",
    "f5": b"\x1b[15~",
}

CHAR_DELAY = 0.07


def parse(path):
    """Return (events, snaps, total_seconds).

    events: list of (time_offset_seconds, bytes) key events.
    snaps:  list of (name, time_offset_seconds) screenshot points.
    """
    events = []
    snaps = []
    t = 0.0
    with open(path, encoding="utf-8") as fh:
        for raw in fh:
            line = raw.rstrip("\n")
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            cmd, _, arg = line.partition(" ")
            arg = arg.strip() if cmd != "type" else line[len("type "):]
            if cmd == "wait":
                t += float(arg)
            elif cmd == "type":
                for ch in arg:
                    events.append((t, ch.encode()))
                    t += CHAR_DELAY
            elif cmd == "key":
                events.append((t, KEYS[arg.lower()]))
                t += 0.05
            elif cmd == "ctrl":
                events.append((t, bytes([ord(arg.lower()) & 0x1F])))
                t += 0.05
            elif cmd == "snap":
                snaps.append((arg, t))
            else:
                raise SystemExit(f"unknown command: {line!r}")
    return events, snaps, t


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("script")
    ap.add_argument("--cwd", required=True)
    ap.add_argument("--bin", default=os.path.abspath("target/release/svnui"))
    ap.add_argument("--cols", type=int, default=110)
    ap.add_argument("--rows", type=int, default=32)
    ap.add_argument("--settle", type=float, default=1.2,
                    help="extra record time after the last event")
    ap.add_argument("--snaps", help="write snap points to this JSON file")
    args = ap.parse_args()

    events, snaps, total = parse(args.script)
    if args.snaps:
        import json
        with open(args.snaps, "w", encoding="utf-8") as fh:
            json.dump(dict(snaps), fh)
    deadline_extra = args.settle

    def watchdog(_sig, _frame):
        # last resort: never hang the recording session
        try:
            os.kill(pid, signal.SIGKILL)
        except Exception:
            pass
        os._exit(0)

    signal.signal(signal.SIGALRM, watchdog)
    signal.alarm(int(total + deadline_extra) + 15)

    pid, master = os.forkpty()
    if pid == 0:  # child
        os.chdir(args.cwd)
        env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor")
        env.pop("NO_COLOR", None)  # the agent shell may export NO_COLOR
        os.execvpe(args.bin, [args.bin, "."], env)

    fcntl.ioctl(master, termios.TIOCSWINSZ,
                struct.pack("HHHH", args.rows, args.cols, 0, 0))

    start = time.monotonic()
    sent = 0
    child_done = False
    end_at = start + total + deadline_extra
    hard_stop = end_at + 4.0

    while True:
        now = time.monotonic()
        while sent < len(events) and events[sent][0] <= now - start:
            os.write(master, events[sent][1])
            sent += 1

        if child_done and now > end_at:
            break
        if now > hard_stop:
            break

        timeout = 0.05
        r, _, _ = select.select([master], [], [], timeout)
        if r:
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            if not data:
                break
            try:
                os.write(1, data)
            except OSError:
                pass

        # child exit detection (non-blocking)
        done, _ = os.waitpid(pid, os.WNOHANG)
        if done == pid:
            child_done = True
            if now > end_at - deadline_extra:  # app already quit
                end_at = min(end_at, now + 0.4)

    # safety net: make sure the child is gone
    done, _ = os.waitpid(pid, os.WNOHANG)
    if done == 0:
        try:
            os.write(master, b"\x1bq")
            time.sleep(0.5)
        except OSError:
            pass
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                break
            time.sleep(0.3)
            done, _ = os.waitpid(pid, os.WNOHANG)
            if done != 0:
                break


if __name__ == "__main__":
    main()
