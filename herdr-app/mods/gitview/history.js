// The history view: its commits (from `json log`), the selection, the
// "branch only" filter, and how a commit row draws: the subject over
// "sha · when · author", refs (branches, tags) as small pills.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { gv, onArgs } from "./engine.js";
import { FileList } from "./filelist.js";
import { treeMode } from "./shared.js";

export const COMMIT_ROW = 44;

export class History {
  // filter: only the branch's own commits (merge-base..HEAD).
  constructor(filter) {
    this.commits = [];
    this.selected = 0;
    this.filter = filter;
    this.loading = false;
  }

  // where: the working tree's {root, on, base}.
  async load(where) {
    this.loading = true;
    try {
      const args = ["log", "--repo", where.root, "--limit", "200", ...onArgs(where.on)];
      if (this.filter) {
        let base = where.base?.merge_base;
        if (!base) base = (await gv(["status", "--repo", where.root, "--scope", "branch", ...onArgs(where.on)])).base?.merge_base;
        if (base) args.push("--merge-base", base);
        else this.filter = false;
      }
      this.commits = (await gv(args)).commits ?? [];
      this.selected = Math.min(this.selected, Math.max(0, this.commits.length - 1));
    } finally {
      this.loading = false;
    }
  }

  toggleFilter() {
    this.filter = !this.filter;
    this.selected = 0;
  }

  move(delta) {
    const n = this.commits.length;
    if (n) this.selected = Math.min(n - 1, Math.max(0, this.selected + delta));
  }

  current() {
    return this.commits[this.selected] ?? null;
  }
}

// ---- what the list does in it (v: the list's view) ---------------------

export function openLog(v, cx) {
  v.setMode("log", cx, { history: new History(v.scope === "branch") });
}

export function loadLog(v, cx) {
  const where = v.work?.where;
  const history = v.history;
  if (where) v.loads.run(cx, "log", () => history.load(where));
}

export function toggleLogFilter(v, cx) {
  v.history.toggleFilter();
  loadLog(v, cx);
}

export function logMove(v, delta) {
  v.history.move(delta);
  v.reveal(v.history.selected, { margin: COMMIT_ROW });
}

// A commit's files, as a file list of their own; the first one's diff shows.
export function openCommit(v, c, cx) {
  const where = v.work?.where;
  if (!c || !where) return;
  v.loads.run(cx, "commit-files", async (cx) => {
    const r = await gv(["commit-files", "--repo", where.root, "--sha", c.sha, ...onArgs(where.on)]);
    const list = new FileList({ sections: [{ id: "commit", title: "", cached: false, files: r.files ?? [] }] },
      { ...where, base: null, commit: c.sha, short: c.short, subject: c.subject });
    list.rebuild(treeMode());
    v.setMode("commit", cx, { list });
  });
}

// ctx: {t, selected (index), focused, onClick(i, cx)}
export function commitRow(c, i, ctx) {
  const { t } = ctx;
  const on = i === ctx.selected;
  const refs = (c.refs ?? []).map((r) => r.replace(/^HEAD -> /, "")).filter((r) => r !== "HEAD");
  return v_flex()
    .id(`commit-${c.sha}`)
    .h(COMMIT_ROW)
    .flex_none()
    .mx(6)
    .px(10)
    .justify_center()
    .gap(2)
    .rounded(7)
    .cursor_pointer()
    .when(on, (e) => e.bg(ctx.focused ? t.selected : t.hover))
    .when(!on, (e) => e.hover((s) => s.bg(t.hover)))
    .on_click((_e, cx) => ctx.onClick(i, cx))
    .child(
      h_flex()
        .gap(6)
        .items_center()
        .min_w(0)
        .child(div().flex_1().min_w(0).truncate().text_color(t.foreground).child(c.subject || "(no message)"))
        .children(refs.slice(0, 2).map((r) => div().flex_none().max_w(110).truncate().px(5).rounded(5).bg(t.raised).text_size(10.5).text_color(t.accent).child(r))),
    )
    .child(
      h_flex()
        .gap(6)
        .min_w(0)
        .text_size(11)
        .text_color(t.dim)
        .child(div().flex_none().font_family("JetBrains Mono").child(c.short))
        .child(div().flex_none().child(c.when ?? ""))
        .child(div().min_w(0).truncate().child(c.author ?? "")),
    );
}
