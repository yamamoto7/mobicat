#!/usr/bin/env python3
"""Put the recordings side by side, with captions and the keys pressed.

    python3 demo/record/compose.py out/

Reads what record.py wrote and writes out/mobicat-demo.mp4 (1920x1080) and
out/mobicat-demo.gif (the short loop for the README). Needs ffmpeg and Pillow.
"""

import json
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, str(Path(__file__).parent))
from record import TERM_H, TERM_W  # noqa: E402
from timeline import END  # noqa: E402

W, H = 1920, 1080
FPS = 30
BG_TOP, BG_BOTTOM = (16, 20, 28), (8, 10, 14)
TEXT = (232, 236, 242)
MUTED = (150, 160, 172)
ACCENT = (110, 214, 222)  # mobicat's cyan
SANS = "/System/Library/Fonts/SFNS.ttf"
MONO = "/System/Library/Fonts/SFNSMono.ttf"

# Layout
TERM_X, TERM_Y = 822, 92
PHONE_Y, SCREEN_H, BEZEL = 92, 744, 9
PHONES = [  # (name, label, screen width/height ratio)
    ("android", "Pixel 8 · Android", 1080 / 2400),
    ("ios", "iPhone 17 Pro · iOS", 1206 / 2622),
]
CAPTION_Y = 968
GIF_SEGMENTS = [(11.5, 21.0), (34.0, 42.0), (54.5, 62.5)]


def font(path, size, weight=None):
    f = ImageFont.truetype(path, size)
    if weight:
        try:
            f.set_variation_by_axes([weight])
        except Exception:
            pass
    return f


def rounded(size, radius, fill):
    im = Image.new("RGBA", size, (0, 0, 0, 0))
    ImageDraw.Draw(im).rounded_rectangle((0, 0, size[0] - 1, size[1] - 1), radius, fill=fill)
    return im


def background(phone_boxes):
    im = Image.new("RGB", (W, H))
    d = ImageDraw.Draw(im)
    for y in range(H):
        k = y / H
        d.line([(0, y), (W, y)], fill=tuple(int(a + (b - a) * k) for a, b in zip(BG_TOP, BG_BOTTOM)))
    f = font(SANS, 22, 500)
    small = font(SANS, 18, 500)
    for (x, y, w, h), (_, label, _) in zip(phone_boxes, PHONES):
        d.text((x + w / 2, y + h + 16), label, font=f, fill=MUTED, anchor="mt")
        d.text((x + w / 2, y + h + 46), "Shelf · sample app", font=small, fill=(120, 128, 138), anchor="mt")
        # The phone body; the screen is drawn on top by ffmpeg.
        body = rounded((w, h), 54, (36, 40, 48, 255))
        im.paste(body, (x, y), body)
    return im


def caption_png(text, path):
    f = font(SANS, 40, 600)
    tw = f.getlength(text)
    im = Image.new("RGBA", (W, 100), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    pad = 34
    d.rounded_rectangle((W / 2 - tw / 2 - pad, 14, W / 2 + tw / 2 + pad, 86), 36, fill=(255, 255, 255, 236))
    d.text((W / 2, 50), text, font=f, fill=(14, 18, 24), anchor="mm")
    im.save(path)


def key_png(label, path):
    f = font(MONO, 30)
    tw = max(f.getlength(label), 30)
    w, h = int(tw + 44), 58
    im = Image.new("RGBA", (W, 80), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    x = TERM_X + TERM_W - w
    d.rounded_rectangle((x, 8, x + w, 8 + h), 12, fill=(30, 34, 42, 245), outline=ACCENT, width=2)
    d.text((x + w / 2, 8 + h / 2), label, font=f, fill=TEXT, anchor="mm")
    im.save(path)


def card(path, title, lines):
    im = Image.new("RGBA", (W, H), (10, 12, 16, 250))
    d = ImageDraw.Draw(im)
    d.text((W / 2, H / 2 - 60), title, font=font(MONO, 120), fill=ACCENT, anchor="mm")
    y = H / 2 + 60
    for text, mono in lines:
        f = font(MONO, 34) if mono else font(SANS, 40, 500)
        d.text((W / 2, y), text, font=f, fill=TEXT if not mono else MUTED, anchor="mm")
        y += 64
    im.save(path)


def sequence(path, items, blank, end):
    """An ffconcat list: each (start, secs, png) shown in turn, blank between."""
    lines, t = ["ffconcat version 1.0"], 0.0
    for start, secs, png in sorted(items):
        if start > t:
            lines += [f"file '{blank}'", f"duration {start - t:.3f}"]
        lines += [f"file '{png}'", f"duration {secs:.3f}"]
        t = start + secs
    lines += [f"file '{blank}'", f"duration {max(end - t, 0.1):.3f}", f"file '{blank}'"]
    path.write_text("\n".join(lines) + "\n")


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "demo/out").resolve()
    ev = json.loads((out / "events.json").read_text())
    work = out / "compose"
    work.mkdir(exist_ok=True)

    # Phone boxes
    boxes, x = [], 52
    for _, _, ratio in PHONES:
        sw = round(SCREEN_H * ratio / 2) * 2
        boxes.append((x, PHONE_Y, sw + 2 * BEZEL, SCREEN_H + 2 * BEZEL))
        x += sw + 2 * BEZEL + 34
    background(boxes).save(work / "bg.png")
    for i, (bx, by, bw, bh) in enumerate(boxes):
        sw, sh = bw - 2 * BEZEL, bh - 2 * BEZEL
        rounded((sw, sh), 46, (255, 255, 255, 255)).convert("L").save(work / f"mask{i}.png")

    blank = work / "blank.png"
    Image.new("RGBA", (W, 100), (0, 0, 0, 0)).save(blank)
    caps = []
    for i, c in enumerate(ev["captions"]):
        p = work / f"cap{i}.png"
        caption_png(c["text"], p)
        caps.append((c["t"], c["secs"], p))
    sequence(work / "captions.txt", caps, blank, END)

    keys = []
    for i, k in enumerate(ev["keys"]):
        p = work / f"key{i}.png"
        key_png(k["label"], p)
        keys.append((k["t"], 0.8, p))
    # Keys pressed in quick succession replace each other.
    keys.sort()
    keys = [(t, min(s, keys[j + 1][0] - t) if j + 1 < len(keys) else s, p)
            for j, (t, s, p) in enumerate(keys)]
    kblank = work / "kblank.png"
    Image.new("RGBA", (W, 80), (0, 0, 0, 0)).save(kblank)
    sequence(work / "keys.txt", keys, kblank, END)

    card(work / "title.png", "mobicat", [("One log viewer for Android and iOS.", False)])
    card(work / "outro.png", "mobicat", [
        ("cargo install --git https://github.com/yamamoto7/mobicat mobicat", True),
        ("github.com/yamamoto7/mobicat", True),
    ])

    t0 = ev["terminal_start"]
    term_dur = float(subprocess.check_output(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0",
         str(out / "terminal.mp4")]).strip())
    # VHS captures a little slower than real time; stretch it back to the clock.
    stretch = (END + 1.0) / term_dur
    ios_skip = t0 - ev["ios_start"]
    and_skip = t0 - ev["android_start"]

    inputs = [
        ["-loop", "1", "-framerate", str(FPS), "-i", str(work / "bg.png")],          # 0
        ["-i", str(out / "terminal.mp4")],                                             # 1
        ["-i", str(out / "android.webm")],                                             # 2
        ["-i", str(out / "ios.mov")],                                                  # 3
        ["-loop", "1", "-i", str(work / "mask0.png")],                                 # 4
        ["-loop", "1", "-i", str(work / "mask1.png")],                                 # 5
        ["-f", "concat", "-safe", "0", "-i", str(work / "captions.txt")],              # 6
        ["-f", "concat", "-safe", "0", "-i", str(work / "keys.txt")],                  # 7
        ["-loop", "1", "-framerate", str(FPS), "-i", str(work / "title.png")],         # 8
        ["-loop", "1", "-framerate", str(FPS), "-i", str(work / "outro.png")],         # 9
    ]
    (ax, ay, aw, ah), (ix, iy, iw, ih) = boxes
    asw, ash = aw - 2 * BEZEL, ah - 2 * BEZEL
    isw, ish = iw - 2 * BEZEL, ih - 2 * BEZEL
    outro_at = END - 5.0
    graph = f"""
[1:v]setpts=PTS*{stretch:.5f},fps={FPS},tpad=stop_mode=clone:stop_duration=10[term];
[2:v]fps={FPS},trim=start={and_skip:.3f},setpts=PTS-STARTPTS,scale={asw}:{ash},format=rgba,tpad=stop_mode=clone:stop_duration=120[a0];
[a0][4:v]alphamerge[and];
[3:v]fps={FPS},trim=start={ios_skip:.3f},setpts=PTS-STARTPTS,scale={isw}:{ish},format=rgba,tpad=stop_mode=clone:stop_duration=120[i0];
[i0][5:v]alphamerge[ios];
[0:v][and]overlay={ax + BEZEL}:{ay + BEZEL}:shortest=1[v1];
[v1][ios]overlay={ix + BEZEL}:{iy + BEZEL}[v2];
[v2][term]overlay={TERM_X}:{TERM_Y}[v3];
[6:v]format=rgba,fps={FPS}[cap];
[v3][cap]overlay=0:{CAPTION_Y - 50}[v4];
[7:v]format=rgba,fps={FPS}[key];
[v4][key]overlay=0:{TERM_Y + TERM_H + 6}[v5];
[8:v]format=rgba,fade=out:st=2.6:d=0.6:alpha=1,trim=duration=3.3[title];
[v5][title]overlay=0:0:eof_action=pass[v6];
[9:v]format=rgba,trim=duration=6,fade=in:st=0:d=0.6:alpha=1,setpts=PTS+{outro_at}/TB[outro];
[v6][outro]overlay=0:0:eof_action=pass,trim=duration={END},format=yuv420p[out]
"""
    mp4 = out / "mobicat-demo.mp4"
    cmd = ["ffmpeg", "-v", "error", "-y"]
    for i in inputs:
        cmd += i
    cmd += ["-filter_complex", graph.replace("\n", ""), "-map", "[out]", "-r", str(FPS),
            "-c:v", "libx264", "-preset", "slow", "-crf", "20", "-movflags", "+faststart", str(mp4)]
    subprocess.run(cmd, check=True)
    print(f"wrote {mp4}")

    # The README loop: the most telling moments, smaller.
    parts = "".join(
        f"[0:v]trim={a}:{b},setpts=PTS-STARTPTS[s{i}];" for i, (a, b) in enumerate(GIF_SEGMENTS))
    concat = "".join(f"[s{i}]" for i in range(len(GIF_SEGMENTS)))
    gif = out / "mobicat-demo.gif"
    subprocess.run([
        "ffmpeg", "-v", "error", "-y", "-i", str(mp4), "-filter_complex",
        f"{parts}{concat}concat=n={len(GIF_SEGMENTS)}:v=1:a=0,fps=12,scale=1200:-1:flags=lanczos,"
        "split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];"
        "[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle",
        str(gif)], check=True)
    print(f"wrote {gif} ({gif.stat().st_size / 1e6:.1f} MB)")


if __name__ == "__main__":
    main()
