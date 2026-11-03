"""spot.py PID W H: where to put PID's window (W×H) so that as much of it as
possible is not under another app's ordinary window (layer 0): macOS draws a
window only while some of it shows, and the private app is never raised.
Prints "X Y VISIBLE DISPLAY" (VISIBLE: the share of sample points that
show, 0..1; DISPLAY: the main display's UUID, as window.json names it); the
best spot with the whole window on the screen wins, the default {40, 60} on
a tie. Reads the window list only."""
import sys

import Quartz
from AppKit import NSScreen

pid, W, H = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
front = []
for w in Quartz.CGWindowListCopyWindowInfo(Quartz.kCGWindowListOptionOnScreenOnly, Quartz.kCGNullWindowID):
    if w.get("kCGWindowOwnerPID") == pid:
        continue
    if w.get("kCGWindowLayer", 0) == 0 and w.get("kCGWindowAlpha", 1) > 0:
        b = w["kCGWindowBounds"]
        front.append((b["X"], b["Y"], b["X"] + b["Width"], b["Y"] + b["Height"]))
f = NSScreen.mainScreen().frame()
SW, SH = int(f.size.width), int(f.size.height)
step = 40
pts = [(dx, dy) for dx in range(0, W, step) for dy in range(0, H, step)]


def visible(x, y):
    n = 0
    for dx, dy in pts:
        sx, sy = x + dx, y + dy
        if sx < SW and sy < SH and not any(a <= sx < c and b <= sy < d for a, b, c, d in front):
            n += 1
    return n / len(pts)


best = (visible(40, 60), 40, 60)
for x in range(0, max(1, SW - W + 1), step):
    for y in range(30, max(31, SH - H + 1), step):
        v = visible(x, y)
        if v > best[0]:
            best = (v, x, y)
def display_uuid():
    import ctypes, ctypes.util
    try:
        cs = ctypes.CDLL("/System/Library/Frameworks/ColorSync.framework/ColorSync")
        cf = ctypes.CDLL(ctypes.util.find_library("CoreFoundation"))
        cs.CGDisplayCreateUUIDFromDisplayID.restype = ctypes.c_void_p
        cs.CGDisplayCreateUUIDFromDisplayID.argtypes = [ctypes.c_uint32]
        cf.CFUUIDCreateString.restype = ctypes.c_void_p
        cf.CFUUIDCreateString.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        cf.CFStringGetCString.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_long, ctypes.c_uint32]
        u = cs.CGDisplayCreateUUIDFromDisplayID(Quartz.CGMainDisplayID())
        buf = ctypes.create_string_buffer(64)
        cf.CFStringGetCString(cf.CFUUIDCreateString(None, u), buf, 64, 0x08000100)
        return buf.value.decode().lower()
    except Exception:
        return "-"


print(best[1], best[2], f"{best[0]:.2f}", display_uuid())
