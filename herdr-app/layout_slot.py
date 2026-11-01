#!/usr/bin/env python3
"""Point one block of the shown Herdr app layout at a mod, or put it back.

    layout_slot.py install   <app home> <slot id> <mod> <herdr-app>
    layout_slot.py uninstall <app home> <slot id> <mod> <herdr-app>

The same mechanism herdr-nvim's installer uses (src/app/setup.rs there), so
the two install and uninstall in either order:

- Herdr's own layouts (`installed.json` lists them as `layout:<name>`) refuse
  content changes and are replaced on every start: a shipped shown layout is
  cloned once as `<name>-mods` (`layout.clone`), or switched to when that
  clone exists. A personal layout is edited in place.
- Only the slot changes: `content = {mod, props: {replaces: <old content>}}`.
- Every write re-reads the file, edits it, and renames a temp file into place
  only if the file is unchanged since the read; otherwise it reads again.
- Uninstall puts `props.replaces` back in every personal layout, then
  deletes a `<name>-mods` clone that matches its origin apart from
  `cloned_from` (switching back first when it's shown), so whichever
  uninstall runs last removes the clone.
"""

import json
import os
import subprocess
import sys


def app(app_bin, home, *args):
    env = dict(os.environ, HERDR_APP_HOME=home)
    out = subprocess.run([app_bin, *args], env=env, capture_output=True, text=True)
    if out.returncode != 0:
        raise SystemExit(f"layout: herdr-app {' '.join(args[:2])} failed: {out.stdout}{out.stderr}".strip())
    return json.loads(out.stdout or "{}")


def shipped(home, name):
    try:
        with open(os.path.join(home, "installed.json")) as f:
            return f"layout:{name}" in json.load(f)
    except (OSError, ValueError):
        return False


def layout_path(home, name):
    return os.path.join(home, "layouts", f"{name}.json")


def find(node, node_id):
    if node.get("id") == node_id:
        return node
    for child in node.get("children", []):
        hit = find(child, node_id)
        if hit:
            return hit
    return None


def walk(node, visit):
    visit(node)
    for child in node.get("children", []):
        walk(child, visit)


def edit_json(path, edit):
    """Read, change and replace `path` whole, unless it changed meanwhile
    (then read again). `edit(doc)` returns whether it changed anything."""
    for _ in range(5):
        with open(path, "rb") as f:
            before = f.read()
        doc = json.loads(before)
        if not edit(doc):
            return False
        tmp = os.path.join(os.path.dirname(path), f".{os.path.basename(path)}.gitview.tmp")
        with open(tmp, "w") as f:
            f.write(json.dumps(doc, indent=2, ensure_ascii=False) + "\n")
        with open(path, "rb") as f:
            if f.read() != before:
                os.remove(tmp)
                continue
        os.replace(tmp, path)
        return True
    raise SystemExit(f"layout: {path} keeps changing; try again")


def install(home, slot, mod, app_bin):
    shown = app(app_bin, home, "status")["layout"]
    target = shown
    if shipped(home, shown):
        target = f"{shown}-mods"
        if os.path.exists(layout_path(home, target)):
            app(app_bin, home, "do", "layout.switch", json.dumps({"name": target}))
        else:
            app(app_bin, home, "do", "layout.clone", json.dumps({"name": shown, "as": target}))
            print(f"layout: cloned {shown} as {target} (Herdr's own layouts can't be edited)")

    def patch(doc):
        node = find(doc, slot)
        if node is None:
            raise SystemExit(f"layout: {target} has no `{slot}` block; use the keys instead")
        old = node.get("content")
        if (old or {}).get("mod") == mod:
            return False
        node["content"] = {"mod": mod, "props": {"replaces": old}}
        return True

    changed = edit_json(layout_path(home, target), patch)
    print(f"layout: {target}: `{slot}` {'now shows' if changed else 'already shows'} {mod}")


def uninstall(home, slot, mod, app_bin):
    try:
        shown = app(app_bin, home, "status")["layout"]
    except SystemExit:
        shown = None
    folder = os.path.join(home, "layouts")
    for entry in sorted(os.listdir(folder)):
        name, ext = os.path.splitext(entry)
        if ext != ".json" or shipped(home, name):
            continue
        path = os.path.join(folder, entry)

        def restore(doc):
            changed = []

            def visit(node):
                content = node.get("content") or {}
                if content.get("mod") != mod:
                    return
                node["content"] = (content.get("props") or {}).get("replaces") or {"mod": slot}
                changed.append(True)

            walk(doc, visit)
            return bool(changed)

        if edit_json(path, restore):
            print(f"layout: {name}: `{slot}` shows what it showed before again")
        with open(path) as f:
            doc = json.load(f)
        origin = (doc.get("cloned_from") or {}).get("name")
        if not origin or not name.endswith("-mods"):
            continue
        try:
            with open(layout_path(home, origin)) as f:
                original = json.load(f)
        except (OSError, ValueError):
            continue
        doc.pop("cloned_from", None)
        if doc != original:
            continue  # another mod still patches it
        if shown == name:
            app(app_bin, home, "do", "layout.switch", json.dumps({"name": origin}))
        os.remove(path)
        state = os.path.join(folder, ".state", f"{name}.json")
        if os.path.exists(state):
            os.remove(state)
        print(f"layout: removed {name}, back to {origin}")


def main():
    if len(sys.argv) != 6 or sys.argv[1] not in ("install", "uninstall"):
        raise SystemExit(__doc__)
    op, home, slot, mod, app_bin = sys.argv[1:]
    (install if op == "install" else uninstall)(home, slot, mod, app_bin)


if __name__ == "__main__":
    main()
