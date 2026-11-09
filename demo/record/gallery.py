"""Render Product Hunt gallery images (1270x760) from frames of the demo video.

Usage: python3 demo/record/gallery.py OUT_DIR
Needs demo/out/mobicat-demo.mp4 (run the recorder first) and ffmpeg.
"""
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

W, H = 1270, 760
TEXT, MUTED, ACCENT = (232, 236, 242), (150, 160, 172), (110, 214, 222)
SANS = "/System/Library/Fonts/SFNS.ttf"
VIDEO = Path(__file__).resolve().parents[1] / "out" / "mobicat-demo.mp4"

SLIDES = [  # (seconds into the video, heading, subheading)
    (18, "One log viewer for Android and iOS", "Your app's logs only, from a Pixel and an iPhone simulator at once"),
    (60, "Crashes stand out, with the whole stack trace", "Kotlin exceptions and Swift fatal errors, even on the phone you are not watching"),
    (26, "One filter for both platforms", "Android Studio style queries: tag:, level:, os:, is:crash, as you type"),
    (84, "Your coding agent gets the same logs", "--last 5m prints recent logs and exits; --json for scripts"),
]


def font(path, size, weight=None):
    f = ImageFont.truetype(path, size)
    if weight:
        try:
            f.set_variation_by_axes([weight])
        except Exception:
            pass
    return f


def slide(frame, heading, sub, out):
    im = Image.new("RGB", (W, H))
    d = ImageDraw.Draw(im)
    for y in range(H):
        k = y / H
        d.line([(0, y), (W, y)], fill=tuple(int(a + (b - a) * k) for a, b in zip((16, 20, 28), (8, 10, 14))))
    d.text((W / 2, 58), heading, font=font(SANS, 44, 650), fill=TEXT, anchor="mm")
    d.text((W / 2, 108), sub, font=font(SANS, 24, 500), fill=MUTED, anchor="mm")
    shot = Image.open(frame).convert("RGB").crop((40, 88, 1890, 925))
    sh = H - 150 - 24
    sw = int(shot.width * sh / shot.height)
    if sw > W - 48:
        sw = W - 48
        sh = int(shot.height * sw / shot.width)
    im.paste(shot.resize((sw, sh), Image.LANCZOS), ((W - sw) // 2, 150))
    im.save(out)


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    for i, (t, heading, sub) in enumerate(SLIDES, 1):
        frame = out / f"frame{i}.png"
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-ss", str(t), "-i", str(VIDEO), "-frames:v", "1", str(frame)], check=True)
        slide(frame, heading, sub, out / f"gallery-{i}.png")
        frame.unlink()


if __name__ == "__main__":
    main()
