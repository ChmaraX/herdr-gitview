# A stand-in coding agent for the gitview e2e: reports an idle status (OSC 7501)
# like real agents, turns on bracketed paste (not with DUMMY_PLAIN=1: a
# program that can't take multi-line text), and shows what it receives:
# the text sitting in its input, and every submitted (Enter) message.
# Everything it reads is appended to $DUMMY_LOG, and its input and submitted
# messages are kept in $DUMMY_LOG.json for the e2e to check.
import json, os, sys, tty

LOG = os.environ.get("DUMMY_LOG", "/tmp/dummy-agent.log")
tty.setraw(0)
out = sys.stdout
PLAIN = os.environ.get("DUMMY_PLAIN") == "1"
out.write(("" if PLAIN else "\x1b[?2004h") + "\x1b]7501;state=idle:app=dummy\x1b\\")
pending, submitted, raw = "", [], b""

def draw():
    tmp = LOG + ".json.tmp"
    with open(tmp, "w") as f:
        json.dump({"input": pending, "submitted": submitted}, f)
    os.replace(tmp, LOG + ".json")
    out.write("\x1b[2J\x1b[H")
    out.write(f"dummy agent - bracketed paste {'off' if PLAIN else 'on'}\r\n\r\n")
    for i, msg in enumerate(submitted, 1):
        out.write(f"SUBMITTED {i}:\r\n")
        for line in msg.split("\n"):
            out.write(f"  | {line}\r\n")
    out.write("\r\nINPUT (not submitted):\r\n")
    for line in pending.split("\n"):
        out.write(f"  > {line}\r\n")
    out.flush()

draw()
with open(LOG, "ab") as log:
    while True:
        chunk = os.read(0, 65536)
        if not chunk:
            break
        log.write(chunk)
        log.flush()
        raw += chunk
        while raw:
            if raw.startswith(b"\x1b[200~"):
                end = raw.find(b"\x1b[201~")
                if end < 0:
                    break  # rest of the paste still coming
                pending += raw[6:end].decode("utf-8", "replace").replace("\r\n", "\n").replace("\r", "\n")
                raw = raw[end + 6:]
            elif raw[:1] == b"\r":
                submitted.append(pending)
                pending, raw = "", raw[1:]
            elif raw[:1] == b"\x03":
                sys.exit(0)
            else:
                pending += raw[:1].decode("latin-1")
                raw = raw[1:]
        draw()
