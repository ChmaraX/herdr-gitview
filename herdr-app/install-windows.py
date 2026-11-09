"""Install gitview into the Herdr desktop app on Windows: install.sh's steps.

    python -I herdr-app/install-windows.py [--binary PATH] [--uninstall]

Builds herdr-gitview.exe (cargo build --release, honouring CARGO_TARGET_DIR)
unless --binary, copies the mods (resolving the repo's symlinks, which a
Windows checkout keeps as small text files), writes mods/gitview/config.json
(with "env": the binary's own `env` mode, as Windows has no /usr/bin/env),
checks every mod, swaps changed ones in with one rename each, then runs
`herdr-gitview app-setup install` with keys.windows.json.
Home: $HERDR_APP_HOME, else ~/.config/herdr-app.
"""

import filecmp
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODS_SRC = ROOT / "herdr-app" / "mods"
KEYS = ROOT / "herdr-app" / "keys.windows.json"
HOME = Path(os.environ.get("HERDR_APP_HOME") or Path.home() / ".config" / "herdr-app")


def die(msg):
    sys.exit(f"gitview: {msg}")


def say(msg):
    print(f"gitview: {msg}")


def ours(d: Path):
    if not d.exists():
        return True
    try:
        return json.loads((d / "mod.json").read_text(encoding="utf-8")).get("title", "").startswith("gitview")
    except Exception:
        return False


def symlinks():
    """Repo paths git records as symlinks (mode 120000)."""
    out = subprocess.run(["git", "-C", str(ROOT), "ls-files", "-s", "herdr-app/mods"],
                         capture_output=True, text=True, check=True).stdout
    return {line.split("\t", 1)[1] for line in out.splitlines() if line.startswith("120000")}


def copy_mod(name, dest: Path, links):
    src = MODS_SRC / name
    for f in src.rglob("*"):
        if f.is_dir():
            continue
        rel = f.relative_to(ROOT).as_posix()
        target = f
        if rel in links and not f.is_symlink():
            target = (f.parent / f.read_text(encoding="utf-8").strip()).resolve()
        out = dest / f.relative_to(src)
        out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(target, out)


def same_tree(a: Path, b: Path):
    def files(d):
        return {p.relative_to(d).as_posix() for p in d.rglob("*") if p.is_file()
                and not any(part.startswith(".") for part in p.relative_to(d).parts)
                and p.name != "jsconfig.json" and not p.name.endswith(".d.ts")}
    fa, fb = files(a), files(b)
    return fa == fb and all(filecmp.cmp(a / f, b / f, shallow=False) for f in fa)


def build(args):
    if "--binary" in args:
        return Path(args[args.index("--binary") + 1])
    say("building (cargo build --release)…")
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    return target / "release" / "herdr-gitview.exe"


def main(args):
    if not HOME.is_dir():
        die(f"no Herdr app home at {HOME}")
    mods = sorted(d.name for d in MODS_SRC.iterdir() if (d / "mod.json").is_file())
    bin_src = build(args)
    if not bin_src.is_file():
        die(f"no binary at {bin_src}")

    if "--uninstall" in args:
        subprocess.run([str(bin_src), "app-setup", "uninstall", "--home", str(HOME), "--keys", str(KEYS)], check=True)
        for d in HOME.glob("mods/gitview*"):
            if d.is_dir() and ours(d):
                shutil.rmtree(d)
        say(f"uninstalled from {HOME}")
        return

    app = shutil.which("herdr-app") or die("herdr-app not on PATH")
    hc = shutil.which("hc") or die("hc not on PATH")
    if subprocess.run([app, "status"], capture_output=True).returncode != 0:
        die(f"start the Herdr app first (HERDR_APP_HOME={HOME})")
    for m in mods:
        if not ours(HOME / "mods" / m):
            die(f"{HOME / 'mods' / m} exists and isn't gitview's; not overwriting it")

    stage = HOME / "mods" / ".gitview-install"
    shutil.rmtree(stage, ignore_errors=True)
    links = symlinks()
    for m in mods:
        copy_mod(m, stage / m, links)
    bin_dest = HOME / "mods" / "gitview" / "bin" / "herdr-gitview.exe"
    (stage / "gitview" / "bin").mkdir(parents=True, exist_ok=True)
    shutil.copyfile(bin_src, stage / "gitview" / "bin" / "herdr-gitview.exe")
    cfg = {"bin": str(bin_dest), "herdr_app": app, "hc": hc, "env": [str(bin_dest), "env"]}
    (stage / "gitview" / "config.json").write_text(json.dumps(cfg, indent=2) + "\n", encoding="utf-8")
    for m in mods:
        r = subprocess.run([app, "check", str(stage / m)], capture_output=True, text=True)
        if r.returncode != 0:
            die(f"herdr-app check failed for {m}:\n{r.stdout}{r.stderr}")

    changed = []
    for m in mods:
        cur = HOME / "mods" / m
        if cur.is_dir() and same_tree(stage / m, cur):
            continue
        changed.append(m)
        if cur.is_dir():
            # A running exe can't be overwritten, but its folder can be renamed.
            cur.rename(stage / f".old-{m}")
        (stage / m).rename(cur)
    shutil.rmtree(stage, ignore_errors=True)
    say(f"mods: {' '.join(changed) or 'none changed'}")

    subprocess.run([str(bin_dest), "app-setup", "install", "--home", str(HOME), "--app", app, "--keys", str(KEYS)], check=True)
    say(f"installed into {HOME} (herdr-app: {app})")


if __name__ == "__main__":
    main(sys.argv[1:])
