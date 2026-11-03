// The diff pane's Preview: a file shown as what it is, not as a diff:
// markdown rendered, images and SVGs drawn. One side of the file comes from
// the engine (`json file`): the side the diff's new version comes from (the
// worktree for a change, the index for a staged one, the commit for a
// history file, the worktree for the branch's diff); a deleted file shows
// its old version; a changed image shows before and after.
//
// Images are written by the engine into the cache under the app home
// (`mods/gitview-diff/.cache`, hidden, so saving there reloads no mod),
// named by content, and drawn with image() by their path relative to the
// app home, as Herdr's own mods do. The cache keeps the newest 50 files and
// 50 MB.

import { div, image } from "gpui-kit";
import { h_flex, v_flex, TextView } from "gpui-base";
import { appHome } from "herdr";
import * as fs from "fs/promises";
import { gv, onArgs } from "./engine.js";
import { pill } from "./chrome.js";

const KINDS = {
  md: "markdown", markdown: "markdown", mdx: "markdown",
  png: "image", jpg: "image", jpeg: "image", gif: "image", webp: "image", bmp: "image", ico: "image", svg: "image",
  html: "html", htm: "html",
};
const CACHE = "mods/gitview-diff/.cache";
const KEEP = { files: 50, bytes: 50 * 1024 * 1024 };

// What a view's file previews as: "markdown", "image", "html", or null.
export function previewKind(view) {
  const ext = /\.([^./]+)$/.exec(view?.path ?? "")?.[1]?.toLowerCase();
  return (ext && KINDS[ext]) || null;
}

// The sides to show: `side` (the file as it is now, or its old version when
// deleted) and, for a changed image, `before`.
function sidesOf(v, kind) {
  const [after, before] = v.commit
    ? [`commit:${v.commit}`, `parent:${v.commit}`]
    : v.section === "branch"
      ? ["worktree", v.base?.merge_base ? `base:${v.base.merge_base}` : null]
      : v.cached
        ? ["index", "head"]
        : ["worktree", "index"];
  if (v.kind === "deleted") return { side: before, deleted: true };
  const changed = kind === "image" && v.kind !== "added" && v.kind !== "untracked";
  return { side: after, before: changed ? before : null };
}

const SIDE_NAMES = { worktree: "worktree", index: "index", head: "HEAD" };
const sideName = (s) => SIDE_NAMES[s] ?? (s?.startsWith("commit:") ? "this commit" : s?.startsWith("parent:") ? "the commit before" : "the base");

function reason(r) {
  if (r.kind === "too_big") return `too big to preview (${(r.bytes / 1048576).toFixed(1)} MB)`;
  if (r.kind === "missing") return "not there on that side";
  if (r.kind === "binary") return "not text";
  return `can't preview a ${r.kind}`;
}

// `had`: what this side gave last time; when the engine says it's the same
// (`--unless` its hash), that comes back as it is.
async function fetchSide(v, path, side, want, had) {
  const unless = had?.hash ? ["--unless", had.hash] : [];
  const r = await gv(["file", "--repo", v.root, "--path", path, "--side", side, "--out", `${appHome()}/${CACHE}`, ...unless, ...onArgs(v.on)]);
  if (r.same) return had;
  if (r.kind !== want) throw new Error(`preview: ${reason(r)}`);
  if (want === "image") r.src = r.path.startsWith(`${appHome()}/`) ? r.path.slice(appHome().length + 1) : r.path;
  return r;
}

// Loads what the preview shows: {kind, text?, after?, before?, deleted,
// sides}. Rejects with the reason it can't (the pane shows the diff then).
// `had`: this file's preview from before; what's unchanged is reused
// (`same` true when nothing changed).
export async function loadPreview(v, had = null) {
  const kind = previewKind(v);
  const { side, before, deleted } = sidesOf(v, kind);
  if (!side) throw new Error("preview: no side to show");
  const path = deleted ? v.orig ?? v.path : v.path;
  if (kind !== "image") {
    const r = await fetchSide(v, path, side, "text", had?.textReply);
    return { kind, text: r.text, textReply: r, deleted: Boolean(deleted), side, same: r === had?.textReply };
  }
  const after = await fetchSide(v, path, side, "image", had?.after);
  let old = null;
  if (before) {
    try {
      old = await fetchSide(v, v.orig ?? v.path, before, "image", had?.before);
    } catch {
      old = null; // no before (new on that side): after only
    }
  }
  prune([after.path, old?.path].filter(Boolean));
  return { kind, after, before: old, deleted: Boolean(deleted), side, beforeSide: before, same: after === had?.after && old === (had?.before ?? null) };
}

// The newest KEEP files and bytes stay (`keep`: the ones on screen).
async function prune(keep) {
  try {
    const dir = `${appHome()}/${CACHE}`;
    const files = [];
    for (const name of await fs.readdir(dir)) {
      const st = await fs.stat(`${dir}/${name}`);
      files.push({ path: `${dir}/${name}`, at: st.mtimeMs, bytes: st.size });
    }
    files.sort((a, b) => b.at - a.at);
    let bytes = 0;
    for (const [i, f] of files.entries()) {
      bytes += f.bytes;
      if ((i >= KEEP.files || bytes > KEEP.bytes) && !keep.includes(f.path)) await fs.unlink(f.path);
    }
  } catch (e) {
    console.warn(`gitview-diff: cache: ${e?.message ?? e}`);
  }
}

// The body: ctx {t, uiFont, narrow}.
export function previewBody(p, ctx) {
  const { t } = ctx;
  const tag = p.deleted ? pill(t, "deleted") : null;
  if (p.kind === "image") {
    const shot = (img, caption) =>
      v_flex()
        .flex_1()
        .min_w(0)
        .min_h(0)
        .gap(6)
        .child(h_flex().gap(6).items_center().child(div().text_size(11).text_color(t.dim).child(caption)).children([tag].filter(Boolean)))
        .child(div().flex_1().min_h(0).w_full().child(image(img.src).size_full()));
    const now = shot(p.after, p.before ? `after · ${sideName(p.side)}` : p.deleted ? `as it was · ${sideName(p.side)}` : sideName(p.side));
    const box = ctx.narrow ? v_flex() : h_flex();
    return box
      .id("gitview-preview")
      .flex_1()
      .min_h(0)
      .p(14)
      .gap(14)
      .children(p.before ? [shot(p.before, `before · ${sideName(p.beforeSide)}`), now] : [now]);
  }
  const text = p.kind === "html" ? TextView.html("gitview-preview-text", p.text) : TextView.markdown("gitview-preview-text", p.text);
  return v_flex()
    .id("gitview-preview")
    .flex_1()
    .min_h(0)
    .px(18)
    .py(12)
    .gap(8)
    .font_family(ctx.uiFont)
    .text_size(13)
    .text_color(t.foreground)
    .children([tag].filter(Boolean))
    .child(text.selectable(true).scrollable(true).flex_1().min_h(0));
}
