// A list of changed files: the working tree's (its status) or one commit's.
// The same rows, selection and folding either way; `where` says which repo,
// on which machine, and for a commit which one: it was loaded with the
// status, so an op always goes to the repo the rows came from.

import { entryOf, fileOrder, listRows, parentDirs, viewOf } from "./model.js";

// Where a row is, as the selection keeps it: {section, path} (a folder's
// path ends in "/"). Section headers aren't selectable.
const spotOf = (row) => (row && row.kind !== "header" ? { section: row.section.id, path: row.kind === "file" ? row.file.path : row.path } : null);
const isAt = (row, spot) =>
  Boolean(spot) && row.kind !== "header" && row.section.id === spot.section && (row.kind === "file" ? row.file.path : row.path) === spot.path;

export class FileList {
  // status: the engine's `status` (or `{sections}` for a commit);
  // where: {root, on, base, commit?, short?, subject?}.
  constructor(status, where) {
    this.status = status;
    this.where = where;
    this.selected = null; // {section, path}
    this.collapsed = new Set();
    this.rows = [];
  }

  rebuild(tree) {
    this.tree = tree;
    this.rows = listRows(this.status, { tree, collapsed: this.collapsed });
  }

  // A new status for the same list: the same row, else the same file in its
  // new section (just staged), else (it went) the file now where it was,
  // else the last one.
  update(status, where) {
    const was = this.selected;
    const index = this.selectedIndex();
    this.status = status;
    this.where = where;
    this.rebuild(this.tree);
    if (!was || this.row()) return;
    const folder = was.path.endsWith("/");
    const moved = folder ? null : this.rows.find((r) => r.kind === "file" && r.file.path === was.path);
    const wanted = (r) => (folder ? r.kind !== "header" : r.kind === "file");
    const near = this.rows.slice(Math.max(0, index)).find(wanted) ?? this.rows.findLast(wanted);
    this.select(moved ?? near ?? null);
  }

  select(row) {
    this.selected = spotOf(row);
  }

  isSelected(row) {
    return isAt(row, this.selected);
  }

  files() {
    return this.status?.sections.reduce((n, s) => n + s.files.length, 0) ?? 0;
  }

  index(row) {
    return row ? this.rows.findIndex((r) => r.key === row.key) : -1;
  }

  selectedIndex() {
    return this.rows.findIndex((r) => isAt(r, this.selected));
  }

  row() {
    return this.rows.find((r) => isAt(r, this.selected)) ?? null;
  }

  // What the diff pane shows for this list: the selected file, else the
  // first one (null: no files).
  shownFile() {
    const row = this.row();
    return row?.kind === "file" ? row : this.rows.find((r) => r.kind === "file") ?? null;
  }

  // The selected folder (tree mode) is gone from the rows (flat): the
  // first file under it in its section.
  selectIntoFolder() {
    const s = this.selected;
    if (!s?.path.endsWith("/") || this.row()) return;
    this.select(this.rows.find((r) => r.kind === "file" && r.section.id === s.section && r.file.path.startsWith(s.path)) ?? null);
  }

  // The files a row stands for: a header's or a folder's, as listed.
  filesOf(row) {
    if (row.kind === "file") return [row.file.path];
    const files = row.section.files.map((f) => f.path);
    return row.kind === "dir" ? files.filter((p) => p.startsWith(row.path)) : files;
  }
  // The row a script names: a file path, "folder/", "section:<id>", or
  // "in:<section>:<path or folder/>" for the one in that section.
  find(what) {
    if (what.startsWith("in:")) {
      const [, id, ...rest] = what.split(":");
      const path = rest.join(":");
      return this.rows.find((r) => r.section.id === id && (r.kind === "dir" ? r.path === path : r.kind === "file" && r.file.path === path)) ?? null;
    }
    if (what.startsWith("section:")) return this.rows.find((r) => r.kind === "header" && r.section.id === what.slice(8)) ?? null;
    if (what.endsWith("/")) return this.rows.find((r) => r.kind === "dir" && r.path === what) ?? null;
    return this.rows.find((r) => r.kind === "file" && r.file.path === what) ?? null;
  }

  // A file row, unfolding the folders above it in tree mode.
  findFile(path, section) {
    const match = (r) => r.kind === "file" && r.file.path === path && (!section || r.section.id === section);
    let row = this.rows.find(match);
    if (!row && this.tree) {
      const hidden = listRows(this.status, { tree: true }).find(match);
      if (!hidden) return null;
      for (const d of parentDirs(hidden)) this.collapsed.delete(d);
      this.rebuild(this.tree);
      row = this.rows.find(match);
    }
    return row ?? null;
  }

  // The next/previous selectable row (folders count, headers don't).
  step(delta) {
    const rows = this.rows.filter((r) => r.kind !== "header");
    if (!rows.length) return null;
    const at = rows.findIndex((r) => isAt(r, this.selected));
    if (at < 0) return rows[delta > 0 ? 0 : rows.length - 1];
    return rows[Math.min(rows.length - 1, Math.max(0, at + delta))];
  }

  edge(last) {
    const rows = this.rows.filter((r) => r.kind !== "header");
    return rows.length ? rows[last ? rows.length - 1 : 0] : null;
  }

  toggleFold(row) {
    if (row.collapsed) this.collapsed.delete(row.key);
    else this.collapsed.add(row.key);
    this.rebuild(this.tree);
  }

  // The folder a row sits in (tree mode).
  parent(row) {
    const at = this.rows.indexOf(row);
    for (let i = at - 1; i >= 0; i--) if (this.rows[i].kind === "dir" && this.rows[i].depth < row.depth) return this.rows[i];
    return null;
  }

  // What the diff pane shows for a file row, with this list's order for J/K.
  viewFor(row, line = null) {
    const files = fileOrder(this.status, this.tree).slice(0, 2000).map(entryOf);
    return { ...viewOf(entryOf(row), this.where), by: "list", files, line };
  }
}
