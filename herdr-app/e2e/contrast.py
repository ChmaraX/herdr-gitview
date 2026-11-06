"""Whether a comment on an added line reads in a theme: its contrast with the
line's tint (the theme's ansi2 at 0x26 over its background, as the diff pane
paints `@ins`), WCAG's ratio. Prints what it measured as JSON; exits 1 when
it's under 3.5.

  contrast.py pixels <window shot.png> <theme.json>
      In the app's own frame: the rows painted in the tint, and in their
      right 60 % (past the marker and the line numbers: the comment's text)
      the pixel that stands out most from the tint, which is the text's
      color where a glyph covers a whole pixel (antialiased edges are blends
      of the text and the tint, so they stand out less). Exit 2: no tinted
      rows in the frame.
  contrast.py computed <colors.jsonl> <theme.json>
      colors.mjs's lines: the color the pane's render code gives each
      comment span (and, for comparison, the engine's own color for it).
"""
import json
import sys

MIN = 3.5


def rgb(h):
    h = h.lstrip("#")
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def over(top, alpha, under):
    return tuple(round(t * alpha + u * (1 - alpha)) for t, u in zip(top, under))


def lum(c):
    def f(x):
        x /= 255
        return x / 12.92 if x <= 0.03928 else ((x + 0.055) / 1.055) ** 2.4
    return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])


def ratio(a, b):
    hi, lo = sorted((lum(a), lum(b)), reverse=True)
    return (hi + 0.05) / (lo + 0.05)


def hexs(c):
    return "#%02x%02x%02x" % c


mode, source, theme_file = sys.argv[1:4]
theme = json.load(open(theme_file))
bg = rgb(theme["background"])
tint = over(rgb(theme["ansi"][2]), 0x26 / 255, bg)

if mode == "pixels":
    from PIL import Image

    img = Image.open(source).convert("RGB")
    w, h = img.size
    px = img.load()
    near = lambda c: all(abs(a - b) <= 3 for a, b in zip(c, tint))
    rows = []
    for y in range(h):
        xs = [x for x in range(w) if near(px[x, y])]
        if len(xs) > 100:
            rows.append((y, xs[0], xs[-1]))
    if not rows:
        print(json.dumps({"tint": hexs(tint), "rows": 0}))
        sys.exit(2)
    best, at = None, None
    for y, a, b in rows:
        for x in range(a + int(0.4 * (b - a)), b + 1):
            c = px[x, y]
            r = ratio(c, tint)
            if best is None or r > best:
                best, at = r, (x, y, hexs(c))
    print(json.dumps({"tint": hexs(tint), "tinted_pixel_rows": len(rows), "text": at[2], "at": at[:2], "contrast": round(best, 2)}))
    sys.exit(0 if best >= MIN else 1)

# computed
spans = [json.loads(l) for l in open(source) if l.strip()]
comments = [s for s in spans if s["text"].lstrip("\u00a0 ").startswith("//") or "MMMM" in s["text"]]
if not comments:
    print(json.dumps({"error": "no comment spans on added lines"}))
    sys.exit(1)
out = {}
for s in comments:
    fill = s["bg"] or ""
    alpha = int(fill[7:9], 16) / 255 if len(fill) == 9 else 1
    behind = over(rgb(fill[:7]), alpha, bg) if fill else bg
    # One line per color pair (every comment span here has the same).
    out[(s["fg"], s["drawn"], hexs(behind))] = {"spans": 0, "engine": s["fg"], "drawn": s["drawn"], "behind": hexs(behind),
        "engine_contrast": round(ratio(rgb(s["fg"]), behind), 2), "contrast": round(ratio(rgb(s["drawn"]), behind), 2)}
out = list(out.values())
for o in out:
    o["spans"] = sum(1 for s in comments if s["drawn"] == o["drawn"] and s["fg"] == o["engine"])
print(json.dumps(out))
sys.exit(0 if all(o["contrast"] >= MIN for o in out) else 1)
