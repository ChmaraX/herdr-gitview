// A list of changed files: the working tree's (its status) or one commit's.
// The same rows, selection and folding either way; `where` says which repo,
// on which machine, and for a commit which one: it was loaded with the
// status, so an op always goes to the repo the rows came from.

import { fileOrder, listRows, parentDirs, viewOf } from "./model.js";
import { entryOf } from "./actions.js";

export class FileList {
  // status: the engine's `status` (or `{sections}` for a commit);
  // where: {root, on, base, editor, commit?, short?, subject?}.
  constructor(status, where) {
    this.status = status;
    this.where = where;
    this.selected = null;
    this.collapsed = new Set();
    this.rows = [];
  }

  rebuild(tree) {
    this.tree = tree;
    this.rows = listRows(this.status, { tree, collapsed: this.collapsed });
  }

  // A new status for the same list: the same row, else the same file in its
  // new section (just staged), else whatever is now where it was.
  update(status, where) {
    const previous = { key: this.selected, index: this.index(this.selected) };
    this.status = status;
    this.where = where;
    this.rebuild(this.tree);
    const rows = this.rows.filter((r) => r.kind !== "header");
    if (rows.some((r) => r.key === previous.key)) return;
    const path = previous.key?.slice(previous.key.indexOf(":") + 1);
    const moved = rows.find((r) => r.kind === "file" && r.file.path === path);
    const near = this.rows.slice(Math.max(0, previous.index)).find((r) => r.kind !== "header") ?? rows[rows.length - 1];
    this.selected = (moved ?? (previous.key ? near : null) ?? null)?.key ?? null;
  }

  files() {
    return this.status?.sections.reduce((n, s) => n + s.files.length, 0) ?? 0;
  }

  index(key) {
    return this.rows.findIndex((r) => r.key === key);
  }

  row() {
    return this.rows.find((r) => r.key === this.selected) ?? null;
  }

  // The row a script names: a file path, "folder/", "section:<id>".
  find(what) {
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
    const at = rows.findIndex((r) => r.key === this.selected);
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
