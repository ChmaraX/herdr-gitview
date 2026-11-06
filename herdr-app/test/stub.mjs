// The app's modules for node: `herdr` over an in-memory world the tests
// set (`world`), and gpui-kit/gpui-base as inert element builders (as
// e2e/colors.mjs). Import this first; then the mods' files load as they
// are in the repo.
import { registerHooks } from "node:module";

export const world = { shared: {}, spaces: [], panes: [], focused: null, props: null, runs: [], terminals: [] };
globalThis.__gvWorld = world;

// Fresh state per test.
export function reset({ spaces = [], panes = [], focused = null, props = null, layout = null } = {}) {
  Object.assign(world, { shared: {}, spaces, panes, focused, props, layout, runs: [], terminals: [] });
}

const HERDR = `
const w = globalThis.__gvWorld;
export const shared = (k) => w.shared[k];
export const setShared = (k, v) => { w.shared[k] = v === undefined ? undefined : JSON.parse(JSON.stringify(v)); };
export const spaces = () => w.spaces;
export const panes = () => w.panes;
export const focused = () => w.focused;
export const props = () => w.props;
export const terminals = () => w.terminals;
export const layout = () => w.layout ?? null;
export const run = (name, args) => { w.runs.push([name, args ?? null]); };
export const notify = () => {};
export const theme = () => ({});
export const settings = () => ({});
export const agents = () => [];
export const appHome = () => "/tmp/gv-test-home";
export const exec = async () => { throw new Error("no exec in unit tests"); };
export const watchFolder = () => new Promise(() => {});
export const nextCommand = () => new Promise(() => {});
export const readTextFile = async () => "";
export const openOverlay = () => {};
export const closeOverlay = () => {};
`;
const GPUI = `
const el = () => new Proxy({}, { get: (t, k) => (k === "when" ? (c, f) => (c ? f(el()) : el()) : () => el()) });
const factory = () => el();
export const div = factory, svg = factory, image = factory, h_flex = factory, v_flex = factory;
export const Input = { new: factory }, Textarea = { new: factory }, TextView = { new: factory };
export const InputState = { new: factory }, TextareaState = { new: factory };
export class View {}
`;
const url = (src) => `data:text/javascript,${encodeURIComponent(src)}`;
registerHooks({
  resolve(spec, ctx, next) {
    if (spec === "herdr") return { url: url(HERDR), shortCircuit: true };
    if (spec === "gpui-kit" || spec === "gpui-base") return { url: url(GPUI), shortCircuit: true };
    return next(spec, ctx);
  },
});
