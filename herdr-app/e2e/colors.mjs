// The text colors the diff pane gives a diff's spans, from the pane's own
// render code (diffview.js renderRow, as installed), for when the window
// can't be drawn and a pixel can't be probed. gpui-kit's builders are
// replaced by recorders: each span element keeps the color its
// text_color() was given. Prints one JSON line per span of an added row:
// {row, text, fg (the engine's), drawn (the pane's), bg (the row's tint as
// painted, maybe translucent over the theme's background)}.
//
//   node colors.mjs <installed diffview.js> <json diff output> <theme colors json>
import { registerHooks } from "node:module";
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

// An element: every builder call returns it; `when(c, f)` applies f when c;
// text_color, child and children are kept.
const STUB = `
const el = () => {
  const me = { color: null, fill: null, kids: [] };
  const p = new Proxy(me, { get(t, k) {
    if (k in t) return t[k];
    if (k === "when") return (c, f) => (c ? f(p) : p);
    if (k === "text_color") return (c) => { t.color = c; return p; };
    if (k === "bg") return (c) => { t.fill = c; return p; };
    if (k === "child") return (c) => { t.kids.push(c); return p; };
    if (k === "children") return (cs) => { t.kids.push(...cs); return p; };
    return () => p;
  } });
  return p;
};
const factory = () => el();
export const div = factory, svg = factory, h_flex = factory, v_flex = factory;
export const Input = { new: factory }, Textarea = { new: factory };
export const InputState = { new: factory }, TextareaState = { new: factory };
`;
registerHooks({
  resolve(spec, ctx, next) {
    if (spec === "gpui-kit" || spec === "gpui-base") return { url: `data:text/javascript,${encodeURIComponent(STUB)}`, shortCircuit: true };
    return next(spec, ctx);
  },
});

const [diffview, diffFile, themeFile] = process.argv.slice(2);
const { renderRow } = await import(pathToFileURL(diffview).href);
const diff = JSON.parse(readFileSync(diffFile, "utf8"));
const t = JSON.parse(readFileSync(themeFile, "utf8"));
const ctx = { t, cursor: -1, selection: null, focused: false, onDown() {}, onMove() {} };
diff.rows.forEach((row, index) => {
  if (row.k !== "add") return;
  const drawn = renderRow({ row, index }, ctx);
  drawn.kids.forEach((span, i) => {
    const [text, fg] = row.s[i];
    console.log(JSON.stringify({ row: index, text: span.kids.join(""), fg, drawn: span.color, bg: span.fill ?? drawn.fill }));
  });
});
