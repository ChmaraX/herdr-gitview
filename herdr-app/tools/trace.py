#!/usr/bin/env python3
"""gitview's trace, for the lead: what the person's app did, in order.

  trace.py [DIR|FILE ...] [--since 10m] [--around mark|last-warn] [--slow 100]

DIR defaults to ~/.config/herdr-app/data/gitview/trace (every *.jsonl in it:
each mod copy's segments and the engine's engine.jsonl, merged by time).
Prints the timeline (one compact line each; --around: 20 s either side of
the last mark or warn), the slowest execs by op (the mods' ms, the app's
queue time included, and the engine's own), renders, warns and errors, lost
or revived loads, and per input the execs and pane commands it led to.
"""
import argparse, glob, json, os, re, sys, time
from collections import defaultdict

SKIP = {"t", "mod", "copy", "tab", "space", "kind"}


def load(paths):
    files = []
    for p in paths:
        files += sorted(glob.glob(os.path.join(p, "*.jsonl"))) if os.path.isdir(p) else [p]
    lines = []
    for f in files:
        with open(f, errors="replace") as fh:
            for raw in fh:
                try:
                    d = json.loads(raw)
                except ValueError:
                    continue
                if isinstance(d, dict) and "t" in d:
                    lines.append(d)
    return sorted(lines, key=lambda d: d["t"])


def clock(t):
    return time.strftime("%H:%M:%S", time.localtime(t / 1000)) + ".%03d" % (int(t) % 1000)


def short(v, n=90):
    s = v if isinstance(v, str) else json.dumps(v, separators=(",", ":"))
    return s if len(s) <= n else s[: n - 1] + "…"


def compact(d):
    who = d.get("mod", "?") + ("/" + d["copy"] if d.get("copy") else "")
    kind = d.get("kind", "?")
    if kind in ("mark", "snapshot"):
        rest = "panes=%d %s" % (len(d.get("panes", [])), short(d.get("state", {}), 160))
    elif kind == "exec":
        rest = "%s %sms%s%s %s" % (d.get("op"), d.get("ms"), "" if d.get("ok", True) else " FAIL", " same" if d.get("same") else "",
                                  short(d.get("error") or d.get("args") or "", 80))
    else:
        rest = " ".join("%s=%s" % (k, short(v, 60)) for k, v in d.items() if k not in SKIP)
    return "%s %-14s %-8s %s" % (clock(d["t"]), who, kind, rest)


def pct(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(p / 100 * (len(xs) - 1))))] if xs else 0


def ago(s):
    m = re.fullmatch(r"(\d+(?:\.\d+)?)([smhd])", s)
    if not m:
        sys.exit("--since: like 90s, 10m, 2h or 1d")
    return float(m.group(1)) * {"s": 1, "m": 60, "h": 3600, "d": 86400}[m.group(2)] * 1000


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("paths", nargs="*", default=[os.path.expanduser("~/.config/herdr-app/data/gitview/trace")])
    ap.add_argument("--since", help="only the last 10m, 2h, …")
    ap.add_argument("--around", choices=["mark", "last-warn"], help="20 s either side of the last mark / warn")
    ap.add_argument("--slow", type=float, default=100, help="execs at least this many ms are listed (100)")
    a = ap.parse_args()
    lines = load(a.paths)
    if a.since:
        cut = time.time() * 1000 - ago(a.since)
        lines = [d for d in lines if d["t"] >= cut]
    if not lines:
        sys.exit("no trace lines")
    shown = lines
    if a.around:
        want = "mark" if a.around == "mark" else "warn"
        at = [d["t"] for d in lines if d.get("kind") == want]
        if not at:
            sys.exit("no %s in the trace" % want)
        shown = [d for d in lines if abs(d["t"] - at[-1]) <= 20000]
        print("== around the last %s, %s (±20 s)" % (want, clock(at[-1])))
    print("== timeline (%d lines, %s – %s)" % (len(shown), clock(shown[0]["t"]), clock(shown[-1]["t"])))
    for d in shown:
        print(compact(d))

    mods = [d for d in lines if d.get("kind") == "exec" and d.get("mod") != "engine"]
    engine = [d for d in lines if d.get("kind") == "exec" and d.get("mod") == "engine"]
    # The engine's line of a mod's exec: the same op, ending inside it.
    for m in mods:
        start = m.get("start", m["t"] - (m.get("ms") or 0))
        hit = next((e for e in engine if e.get("op") == m.get("op") and start <= e["t"] <= m["t"] + 5 and not e.get("_used")), None)
        if hit:
            hit["_used"] = True
            m["engine_ms"] = hit.get("ms")
    print("\n== execs by op (ms: the mods' p50 / p95 / max · the engine's p50 / max)")
    by = defaultdict(list)
    for m in mods:
        by[m.get("op")].append(m)
    for op, ms in sorted(by.items(), key=lambda kv: -max(x.get("ms") or 0 for x in kv[1])):
        mm = [x.get("ms") or 0 for x in ms]
        em = [x["engine_ms"] for x in ms if x.get("engine_ms") is not None]
        fails = sum(1 for x in ms if not x.get("ok", True))
        print("%-14s n=%-4d %6.0f %6.0f %6.0f · %s%s" % (op, len(ms), pct(mm, 50), pct(mm, 95), max(mm),
              "%5.1f %6.1f" % (pct(em, 50), max(em)) if em else "   -", "  failed %d" % fails if fails else ""))
    slow = [m for m in mods if (m.get("ms") or 0) >= a.slow]
    if slow:
        print("\n== execs over %g ms" % a.slow)
        for m in slow:
            print("%s  (engine %s ms)" % (compact(m), m.get("engine_ms", "-")))

    renders = [d for d in lines if d.get("kind") == "render"]
    if renders:
        print("\n== renders")
        per = defaultdict(lambda: [0, 0.0, 0.0, 0])
        for r in renders:
            p = per[r.get("mod")]
            if "slow" in r:
                p[3] += 1
                p[2] = max(p[2], r["slow"])
            else:
                p[0] += r.get("n", 0)
                p[1] += r.get("ms", 0)
                p[2] = max(p[2], r.get("max", 0))
        for mod, (n, ms, mx, sl) in sorted(per.items()):
            print("%-10s %6d renders, %7.1f ms in all, max %.1f ms, %d over 16 ms" % (mod, n, ms, mx, sl))

    warns = [d for d in lines if d.get("kind") == "warn"]
    lost = [d for d in warns if d.get("lost")]
    print("\n== warns and errors: %d (lost or revived loads: %d)" % (len(warns), len(lost)))
    for d in warns:
        print(compact(d))
    marks = [d for d in lines if d.get("kind") == "mark"]
    if marks:
        print("\n== marks: %d" % len(marks))
        for d in marks:
            print("%s %s/%s %s" % (clock(d["t"]), d.get("mod"), d.get("copy"), short({k: d.get(k) for k in ("panes", "shared", "state")}, 600)))

    # An input, then what it led to (execs and pane commands until the next
    # input, 3 s at most) and the time to the last of them.
    print("\n== interactions (input → what followed, latency to the last exec or pane command)")
    events = [d for d in lines if d.get("mod") != "engine"]
    for i, d in enumerate(events):
        if d.get("kind") != "input":
            continue
        after = []
        for e in events[i + 1:]:
            if e.get("kind") == "input" or e["t"] - d["t"] > 3000:
                break
            if e.get("kind") in ("exec", "pane"):
                after.append(e)
        what = d.get("command") or d.get("key") or d.get("click") or "?"
        led = ", ".join("%s%s" % (e.get("op") or e.get("cmd"), " %sms" % e["ms"] if "ms" in e else "") for e in after) or "-"
        print("%s %-10s %-22s %5s ms  %s" % (clock(d["t"]), d.get("mod"), short(what, 22), after[-1]["t"] - d["t"] if after else "-", short(led, 110)))


if __name__ == "__main__":
    main()
