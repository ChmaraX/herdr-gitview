"""Whether macOS draws a process's window now: `drawn.py PID` exits 0 when
some part of its main window is on screen and not covered by a window in
front of it, 1 when it's fully covered (macOS then doesn't draw it: no new
frames, no layout, so screenshots are stale and mods can't take the
keyboard). It only reads the window list; it never moves or raises
anything. Needs pyobjc (Quartz)."""
import sys

import Quartz

pid = int(sys.argv[1])
windows = Quartz.CGWindowListCopyWindowInfo(Quartz.kCGWindowListOptionOnScreenOnly, Quartz.kCGNullWindowID)
front = []  # windows above ours, front to back
for w in windows:
    b = w.get("kCGWindowBounds")
    rect = (b["X"], b["Y"], b["X"] + b["Width"], b["Y"] + b["Height"])
    if w.get("kCGWindowOwnerPID") == pid and w.get("kCGWindowLayer") == 0 and b["Width"] > 200:
        ours = rect
        break
    if w.get("kCGWindowLayer", 0) >= 0 and w.get("kCGWindowAlpha", 1) > 0:
        front.append(rect)
else:
    sys.exit(1)  # not on screen at all
covered = any(f[0] <= ours[0] and f[1] <= ours[1] and f[2] >= ours[2] and f[3] >= ours[3] for f in front)
sys.exit(1 if covered else 0)
