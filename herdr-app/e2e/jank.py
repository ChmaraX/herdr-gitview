"""jank.py ACTION_DIR: what one recorded action looked like.

ACTION_DIR holds:
- trace.log: the mods' draw lines of the action, JSON (what each copy
  draws, a line each time that changes; telemetry.js Tracer.drew);
- execs.log: one line per `herdr-gitview json` run (start end code args);
- with E2E_FOREGROUND, the distinct frames (f-<ms>.png, `herdr-app shot` in
  a loop, a frame kept only when it differs from the one before).

Prints one JSON object. From the trace (the pane's lines unless said):
- blank: times the body went blank (no rows, no message) with a file shown;
- loading: times a "Loading…" text showed (pane or list);
- no_file: times the pane showed "No file selected";
- mode_flips: Diff/Preview changes for one file;
- spinner_blinks: times a spinner appeared (pane or list);
- trace_header_changes: changes of the pane header's text;
- stale_end: the pane's file at the end isn't the one the list shows;
- execs: json ops run, by op ("status": 2, ...).
From the frames (0 without them):

- frames: the distinct frames;
- cards: the most pane cards seen in one frame (terminal, diff, panel...);
  four or more means a pane briefly squeezed in;
- blank: frames whose diff pane body is all one colour (no text) while a
  diff pane is there: the body blanked;
- header_changes: frames where the diff pane's header band changed;
- terminal_widths: the distinct widths of the first (terminal) card;
- execs: json ops run, by op ("status": 2, ...).

How the panes are found: each pane is a card with a 1 px border, so its
left and right edges are columns of one non-background colour over most of
the window's height. Cards are those edges in pairs, left to right after
the sidebar's divider; the last card is the panel, the one before it the
diff pane (when there are three or more).
"""
import json
import os
import sys
from collections import Counter

from PIL import Image

d = sys.argv[1]
frames = sorted(f for f in os.listdir(d) if f.startswith("f-") and f.endswith(".png"))


def edges(img):
    # Thin vertical lines: a column that differs from the columns 2 px to
    # either side, in the same colour, over most of the height.
    w, h = img.size
    px = img.load()
    y0, y1 = int(h * 0.12), int(h * 0.92)
    ys = range(y0, y1, 4)
    near = lambda p, q: abs(p[0] - q[0]) + abs(p[1] - q[1]) + abs(p[2] - q[2]) < 12
    xs = []
    for x in range(int(w * 0.15), w - 2):
        c = px[x, (y0 + y1) // 2]
        hits = sum(1 for y in ys if near(px[x, y], c) and not near(px[x - 2, y], c) and not near(px[x + 2, y], c))
        if hits > 0.7 * len(ys) and (not xs or x - xs[-1] > 3):
            xs.append(x)
    return xs, None


def cards_of(xs):
    # Pairs of edges (left, right) at least 100 px apart.
    out, i = [], 0
    while i + 1 < len(xs):
        if xs[i + 1] - xs[i] >= 100:
            out.append((xs[i], xs[i + 1]))
            i += 2
        else:
            i += 1
    return out


def top_of(img, x):
    px = img.load()
    c = px[x, img.size[1] // 2]
    y = img.size[1] // 2
    while y > 0 and abs(sum(px[x, y - 1]) - sum(c)) < 12:
        y -= 1
    return y


res = {"frames": len(frames), "cards": 0, "blank": 0, "blank_frames": [], "header_changes": 0, "terminal_widths": []}
last_header = None
widths = []
for f in frames:
    img = Image.open(os.path.join(d, f)).convert("RGB")
    xs, bg = edges(img)
    cards = cards_of(xs)
    res["cards"] = max(res["cards"], len(cards))
    if cards:
        tw = cards[0][1] - cards[0][0]
        if not widths or abs(widths[-1] - tw) > 2:
            widths.append(tw)
    if len(cards) < 3:
        continue
    a, b = cards[-2]
    top = top_of(img, a)
    # The pane's own title bar (~30 px), then gitview's 42 px header, then rows.
    header = img.crop((a + 6, top + 30, b - 6, top + 72))
    hb = header.tobytes()
    if last_header is not None and hb != last_header:
        res["header_changes"] += 1
    last_header = hb
    band = img.crop((a + 8, top + 80, b - 8, min(img.size[1] - 40, top + 200)))
    colours = Counter(band.getdata())
    common = colours.most_common(1)[0][1]
    if common >= 0.995 * band.size[0] * band.size[1]:
        res["blank"] += 1
        res["blank_frames"].append(f)
res["terminal_widths"] = widths

lines = []
tp = os.path.join(d, "trace.log")
if os.path.exists(tp):
    for l in open(tp):
        try:
            lines.append(json.loads(l))
        except ValueError:
            pass
pane = [l for l in lines if l["mod"] == "pane"]
res.update(blank=res["blank"], loading=0, no_file=0, mode_flips=0, spinner_blinks=0, trace_header_changes=0, stale_end=False)
tb = 0
prev = {}
for l in lines:
    p = prev.get((l["mod"], l.get("tab")))
    if l["body"] == "loading-text" and (not p or p["body"] != "loading-text"):
        res["loading"] += 1
    if l["spinner"] and (not p or not p["spinner"]):
        res["spinner_blinks"] += 1
    if l["mod"] == "pane":
        if l["body"] == "blank" and l.get("file") and (not p or p["body"] != "blank"):
            tb += 1
        if l["body"] == "empty-state" and not l.get("file") and (not p or p["body"] != "empty-state" or p.get("file")):
            res["no_file"] += 1
        if p and p.get("file") == l.get("file") and p["mode"] != l["mode"]:
            res["mode_flips"] += 1
        if p and p["header"] != l["header"]:
            res["trace_header_changes"] += 1
    prev[(l["mod"], l.get("tab"))] = l
res["trace_blank"] = tb
if pane:
    last = pane[-1]
    res["stale_end"] = bool(last.get("file") and last.get("shown") and last["file"].split(":", 1)[1].split("@")[0] != last["shown"])
res["traces"] = len(lines)
execs = Counter()
log = os.path.join(d, "execs.log")
if os.path.exists(log):
    for line in open(log):
        parts = line.split()
        if len(parts) > 4:
            execs[parts[4]] += 1
res["execs"] = dict(execs)
print(json.dumps(res))
