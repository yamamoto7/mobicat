#!/usr/bin/env python3
"""Record the demo: the terminal (VHS), the Android emulator and the iOS simulator.

Needs a booted emulator and simulator with the Shelf demo app installed
(see demo/README.md), plus vhs, tmux and a release build of mobicat.

    python3 demo/record/record.py out/

Writes terminal.mp4, android.webm, ios.mov and events.json (when each file
started, and every key and caption) for compose.py.
"""

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from timeline import APP, END, TIMELINE  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
SESSION = "mobicat-demo"
# Terminal size in pixels; compose.py places it at this size.
TERM_W, TERM_H = 1060, 800
TYPE_DELAY = 0.055


def sh(*args, check=True, **kw):
    return subprocess.run(args, check=check, **kw)


def adb(*args):
    sh("adb", *args, check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def simctl(*args):
    sh("xcrun", "simctl", *args, check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def tmux(*args):
    sh("tmux", *args, check=False)


def prepare_phones():
    # Clean status bars.
    simctl("status_bar", "booted", "override", "--time", "9:41", "--batteryState", "charged",
           "--batteryLevel", "100", "--wifiBars", "3", "--cellularBars", "4")
    adb("shell", "settings", "put", "global", "sysui_demo_allowed", "1")
    for extra in (["command", "enter"],
                  ["command", "clock", "-e", "hhmm", "0941"],
                  ["command", "battery", "-e", "level", "100", "-e", "plugged", "false"],
                  ["command", "notifications", "-e", "visible", "false"],
                  ["command", "network", "-e", "wifi", "show", "-e", "level", "4",
                   "-e", "mobile", "show", "-e", "level", "4", "-e", "datatype", "none"]):
        adb("shell", "am", "broadcast", "-a", "com.android.systemui.demo", "-e", *extra)
    # Fresh app state on both phones.
    adb("shell", "am", "force-stop", APP)
    adb("logcat", "-c")
    simctl("terminate", "booted", APP)
    time.sleep(1)
    adb("shell", "am", "start", "-n", f"{APP}/.MainActivity")
    simctl("launch", "booted", APP)
    time.sleep(3)


def prepare_terminal(state_dir: Path, work: Path):
    sh("tmux", "kill-session", "-t", SESSION, check=False, stderr=subprocess.DEVNULL)
    env = {
        "PATH": f"{ROOT / 'target' / 'release'}:{os.environ['PATH']}",
        "XDG_STATE_HOME": str(state_dir),
        "XDG_CONFIG_HOME": str(state_dir / "config"),
        "PS1": r"\[\e[36m\]~/shelf\[\e[0m\] $ ",
        "TERM": "xterm-256color",
        "BASH_SILENCE_DEPRECATION_WARNING": "1",
    }
    args = ["tmux", "-f", "/dev/null", "new-session", "-d", "-s", SESSION, "-c", str(work),
            "env", *(f"{k}={v}" for k, v in env.items()), "bash", "--noprofile", "--norc"]
    sh(*args)
    tmux("set", "-t", SESSION, "status", "off")
    tmux("set", "-t", SESSION, "escape-time", "0")
    tmux("set", "-t", SESSION, "window-size", "latest")
    tmux("send-keys", "-t", SESSION, "clear", "Enter")


def tape(out: Path, marker: Path) -> Path:
    t = out / "terminal.tape"
    t.write_text(f"""Output "{out / 'terminal.mp4'}"
Set Shell "bash"
Set FontFamily "Menlo"
Set FontSize 16
Set LineHeight 1.15
Set Width {TERM_W}
Set Height {TERM_H}
Set Padding 14
Set Margin 0
Set WindowBar Colorful
Set BorderRadius 10
Set Framerate 30
Set Theme "Catppuccin Mocha"
Hide
Type "touch {marker} && tmux attach -t {SESSION}"
Enter
Sleep 400ms
Show
Sleep {END + 1:.1f}s
""")
    return t


def type_text(text):
    for ch in text:
        tmux("send-keys", "-t", SESSION, "-l", ch)
        time.sleep(TYPE_DELAY)


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "demo/out").resolve()
    out.mkdir(parents=True, exist_ok=True)
    for f in ("terminal.mp4", "android.webm", "ios.mov"):
        (out / f).unlink(missing_ok=True)
    state = Path(tempfile.mkdtemp(prefix="mobicat-demo-"))
    work = state / "shelf"
    work.mkdir()
    marker = state / "attached"

    prepare_phones()
    prepare_terminal(state, work)

    events = {"keys": [], "captions": []}
    # Phone recordings start first; their start times are noted.
    ios = subprocess.Popen(["xcrun", "simctl", "io", "booted", "recordVideo", "--codec", "h264",
                            "--force", str(out / "ios.mov")],
                           stderr=subprocess.PIPE, text=True)
    while True:
        line = ios.stderr.readline()
        if not line or "Recording started" in line:
            break
    events["ios_start"] = time.time()
    sh("adb", "emu", "screenrecord", "start", "--time-limit", str(int(END + 20)),
       str(out / "android.webm"), stdout=subprocess.DEVNULL)
    events["android_start"] = time.time()

    vhs = subprocess.Popen(["vhs", str(tape(out, marker))],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    while not marker.exists():
        time.sleep(0.02)
    t0 = time.time() + 0.4  # the tape's Sleep before Show
    events["terminal_start"] = t0

    for at, kind, *rest in TIMELINE:
        delay = t0 + at - time.time()
        if delay > 0:
            time.sleep(delay)
        if kind == "type":
            type_text(rest[0])
        elif kind == "key":
            tmux("send-keys", "-t", SESSION, rest[0])
            events["keys"].append({"t": time.time() - t0, "label": rest[1]})
        elif kind == "android":
            adb("shell", "am", "start", "-a", "android.intent.action.VIEW", "-p", APP,
                "-d", f"'shelf://{rest[0]}'")
        elif kind == "android-launch":
            adb("shell", "am", "start", "-n", f"{APP}/.MainActivity")
        elif kind == "ios":
            simctl("spawn", "booted", "notifyutil", "-p", f"{APP}.{rest[0]}")
        elif kind == "caption":
            events["captions"].append({"t": at, "text": rest[0], "secs": rest[1]})

    vhs.wait()
    sh("adb", "emu", "screenrecord", "stop", check=False, stdout=subprocess.DEVNULL)
    ios.send_signal(2)
    ios.wait()
    tmux("kill-session", "-t", SESSION)
    events["end"] = END
    (out / "events.json").write_text(json.dumps(events, indent=2))
    shutil.rmtree(state, ignore_errors=True)
    print(f"recorded into {out}")


if __name__ == "__main__":
    main()
