import sys
"""Render docs/assets/social-preview.png (GitHub social preview, 1280x640).

Usage: python3 demo/record/social_card.py FRAME.png docs/assets/social-preview.png
FRAME.png is a 1920x1080 frame of the demo video, e.g.
ffmpeg -ss 50 -i demo/out/mobicat-demo.mp4 -frames:v 1 frame.png
"""
from PIL import Image, ImageDraw, ImageFont
src, out = sys.argv[1], sys.argv[2]
W, H = 1280, 640
TEXT=(232,236,242); MUTED=(150,160,172); ACCENT=(110,214,222)
def font(p,s,w=None):
    f=ImageFont.truetype(p,s)
    if w:
        try: f.set_variation_by_axes([w])
        except Exception: pass
    return f
SANS="/System/Library/Fonts/SFNS.ttf"; MONO="/System/Library/Fonts/SFNSMono.ttf"
im=Image.new("RGB",(W,H))
d=ImageDraw.Draw(im)
for y in range(H):
    k=y/H; d.line([(0,y),(W,y)],fill=tuple(int(a+(b-a)*k) for a,b in zip((16,20,28),(8,10,14))))
frame=Image.open(src).convert("RGB").crop((40,88,1890,925))
fh=H-150-22; fw=int(frame.width*fh/frame.height)
frame=frame.resize((fw,fh),Image.LANCZOS)
top=150
im.paste(frame,((W-fw)//2,top))
d.text((48,46),"mobicat",font=font(MONO,64),fill=ACCENT,anchor="lm")
d.text((48+ d.textlength("mobicat",font=font(MONO,64))+28,48),"One log viewer for Android and iOS",font=font(SANS,40,600),fill=TEXT,anchor="lm")
d.text((48,102),"Your app's logs only · crashes with stack traces · same keys on both · agent-ready",font=font(SANS,22,500),fill=MUTED,anchor="lm")
im.save(out)
print(fh, top)
