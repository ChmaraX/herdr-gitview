// What the file list shows, as data: the engine's status (or a commit's
// files) turned into rows, flat or as a folder tree, and the "view" the
// list hands the diff pane for one file. No drawing here. Shared by gitview
// and gitview-diff (linked; install copies it).

// Section headers, folders and files as one list.
// - header: {kind: "header", key: "h:<section>", section}
// - dir: {kind: "dir", key: "d:<section>:<path>/", section, path, name, depth, collapsed, count}
// - file: {kind: "file", key: "<section>:<path>", section, file, depth}
// `tree`: folders as rows (single-folder chains folded into one, like the
// TUI's tree); `collapsed`: dir keys whose contents are hidden.
export function listRows(status, { tree = false, collapsed = new Set() } = {}) {
  const out = [];
  for (const section of status?.sections ?? []) {
    if (section.title) out.push({ kind: "header", key: `h:${section.id}`, section });
    if (!tree) {
      for (const file of section.files) out.push({ kind: "file", key: `${section.id}:${file.path}`, section, file, depth: 0 });
      continue;
    }
    emit(build(section.files), section, 0, "", collapsed, out);
  }
  return out;
}

// Every file in the order the list shows it (folded folders included), for
// next/previous file from the diff pane.
export function fileOrder(status, tree) {
  return listRows(status, { tree }).filter((r) => r.kind === "file");
}

function build(files) {
  const root = { dirs: new Map(), files: [] };
  for (const file of files) {
    const parts = file.path.split("/");
    let node = root;
    for (const dir of parts.slice(0, -1)) {
      if (!node.dirs.has(dir)) node.dirs.set(dir, { dirs: new Map(), files: [] });
      node = node.dirs.get(dir);
    }
    node.files.push(file);
  }
  return root;
}

function count(node) {
  let n = node.files.length;
  for (const d of node.dirs.values()) n += count(d);
  return n;
}

function emit(node, section, depth, prefix, collapsed, out) {
  const dirs = [...node.dirs.entries()].sort(([a], [b]) => a.localeCompare(b));
  for (let [name, child] of dirs) {
    // A folder holding just one folder shows as one row: "src/app/".
    while (child.files.length === 0 && child.dirs.size === 1) {
      const [[inner, next]] = [...child.dirs.entries()];
      name = `${name}/${inner}`;
      child = next;
    }
    const path = `${prefix}${name}/`;
    const key = `d:${section.id}:${path}`;
    const folded = collapsed.has(key);
    out.push({ kind: "dir", key, section, path, name, depth, collapsed: folded, count: count(child) });
    if (!folded) emit(child, section, depth + 1, path, collapsed, out);
  }
  const files = [...node.files].sort((a, b) => a.path.localeCompare(b.path));
  for (const file of files) out.push({ kind: "file", key: `${section.id}:${file.path}`, section, file, depth });
}

// The dir keys a file sits under (to unfold them when it's selected).
export function parentDirs(row) {
  const parts = row.file.path.split("/").slice(0, -1);
  const out = [];
  for (let i = 1; i <= parts.length; i++) out.push(`d:${row.section.id}:${parts.slice(0, i).join("/")}/`);
  return out;
}

// The pane header's scope tag.
function scopeLabel(entry, where) {
  if (where.commit) return `commit ${where.short}`;
  if (entry.section === "branch") return `vs ${where.base?.label ?? "base"}`;
  return entry.cached ? "staged" : "uncommitted";
}

// The header's "+A −D": every file of every section, so a file both staged
// and changed counts twice, as v1's did (one entry per section). A binary
// file has no counts (null) and adds nothing.
export function lineTotals(sections) {
  let add = 0;
  let del = 0;
  for (const s of sections ?? []) {
    for (const f of s.files) {
      add += f.add ?? 0;
      del += f.del ?? 0;
    }
  }
  return { add, del };
}

// One file of a list ({file, section}), as the diff pane needs it.
export function entryOf({ file, section }) {
  return {
    path: file.path,
    orig: file.orig ?? null,
    kind: file.kind,
    letter: file.letter,
    add: file.add,
    del: file.del,
    cached: Boolean(section.cached),
    section: section.id,
    rev: file.rev ?? null,
  };
}

// What the diff pane shows for one file: everything it needs to load the
// diff, stage it and send notes about it, without the list.
// `where`: {root, on, base, commit, short}.
export function viewOf(entry, where) {
  return {
    root: where.root,
    on: where.on ?? null,
    base: entry.section === "branch" ? where.base ?? null : null,
    commit: where.commit ?? null,
    short: where.short ?? null,
    ...entry,
    scope: scopeLabel(entry, where),
  };
}

// The same file, by what identifies it in a view.
export const sameFile = (a, b) =>
  Boolean(a && b) && a.root === b.root && a.on === b.on && a.path === b.path && a.section === b.section && a.commit === b.commit;

// The same file shown the same way: what the pane shows of it (its rev,
// counts, kind, side, scope) unchanged too.
export const sameShown = (a, b) => sameFile(a, b) && ["rev", "add", "del", "kind", "letter", "cached", "scope"].every((k) => a[k] === b[k]);

// `herdr-gitview json diff` arguments for a view.
// `unless`: the hash of the rows shown now (the reply is {same} when they'd
// be the same).
export function diffArgs(view, { dark, unfold, unless = null }) {
  // `--root`: view.root is a status's root (the engine skips looking it up).
  const args = ["diff", "--repo", view.root, "--root", "--path", view.path, "--kind", view.kind, "--theme", dark ? "dark" : "light"];
  if (view.orig) args.push("--orig", view.orig);
  if (view.cached) args.push("--cached");
  if (view.base) args.push("--merge-base", view.base.merge_base, "--base", view.base.label);
  if (view.commit) args.push("--commit", view.commit);
  if (unfold) args.push("--unfold");
  if (unless) args.push("--unless", unless);
  if (view.on) args.push("--on", view.on);
  return args;
}

// Status letter colors, by theme role.
export const LETTER_COLOR = { M: "blocked", A: "ansi2", U: "ansi2", D: "ansi1", R: "ansi4", "!": "ansi1" };

// Syntax colors come in a dark and a light set, by the theme's background.
export function isDark(hex) {
  const n = parseInt(String(hex).slice(1, 7), 16);
  if (Number.isNaN(n)) return true;
  return 0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255) < 128;
}
