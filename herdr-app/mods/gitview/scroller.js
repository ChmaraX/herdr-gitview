// A scrolling list drawn by the mod itself: only the rows on screen are
// built, like a virtual list, but they're ordinary elements, so rows can
// have their own buttons, hover and drag handlers (inside the app's virtual
// lists they can't). The wheel and the thumb move it; code keeps a row in
// view. Shared (linked; install copies it).

import { div } from "gpui-kit";
import { v_flex } from "gpui-base";
import { unzoom } from "./zoom.js";

const BAR = 6;

export class Scroller {
  // `horizontal`: opt-in sideways scrolling (the diff pane; the file list
  // leaves it off and is unchanged). The sideways range is measured from the
  // rows (measureWidth), not estimated: the widest row seen so far plus
  // `pad` px of room after it. It only grows per file (monotonic, so the
  // measure can't oscillate) and resets on another file (resetX).
  constructor(id, { horizontal = false } = {}) {
    this.id = id;
    this.top = 0;
    this.height = 0; // the viewport, in layout px
    this.sizes = null;
    this.offsets = [0];
    this.dragging = null;
    this.horizontal = horizontal;
    this.left = 0; // the sideways offset, in layout px
    this.vw = 0; // the viewport's width, in layout px
    this.contentW = 0; // the widest row measured so far
    this.pad = 0; // room after the widest row (set by the owner)
  }

  // The sideways range: the widest row plus its room, never less than the
  // viewport (so a file that fits never scrolls).
  range() {
    return Math.max(this.contentW + this.pad, this.vw);
  }

  clampX() {
    this.left = Math.max(0, Math.min(this.left, this.range() - this.vw));
  }

  // Room to leave after the widest row (the diff's "2 columns").
  setPad(pad) {
    if (pad === this.pad) return;
    this.pad = pad;
    this.clampX();
  }

  // A drawn row's natural width (layout px). Monotonic: only a wider row
  // grows the range, so measure -> notify -> redraw -> measure can't loop.
  // Returns true when the range grew.
  measureWidth(px, cx) {
    const w = unzoom(px);
    if (!(w > this.contentW)) return false;
    this.contentW = w;
    this.clampX();
    cx?.notify();
    return true;
  }

  // Sideways by `px` (negative: left); returns true when it moved.
  scrollBy(px) {
    const before = this.left;
    this.left += px;
    this.clampX();
    return this.left !== before;
  }

  // Another file: start sideways from 0 with a fresh range.
  resetX() {
    this.left = 0;
    this.contentW = 0;
  }

  // Item heights.
  measure(list) {
    const offsets = new Array(list.length + 1);
    offsets[0] = 0;
    for (let i = 0; i < list.length; i++) offsets[i + 1] = offsets[i] + list[i];
    this.sizes = list;
    this.offsets = offsets;
    this.clamp();
  }

  total() {
    return this.offsets[this.offsets.length - 1];
  }

  clamp() {
    this.top = Math.max(0, Math.min(this.top, this.total() - this.height));
  }

  // The item under a y in the viewport (or -1).
  indexAt(y) {
    const at = this.top + y;
    let lo = 0;
    let hi = this.sizes.length - 1;
    if (hi < 0 || at < 0 || at >= this.total()) return -1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.offsets[mid] <= at) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  // Scroll just enough to show item `i` (with `margin` px around it), or
  // put it in the middle: on the next draw, once the rows are measured.
  reveal(i, { center = false, margin = 0 } = {}) {
    this.pending = { i, center, margin };
  }

  apply({ i, center, margin }) {
    if (i < 0 || i >= this.sizes.length) return;
    const a = this.offsets[i];
    const b = this.offsets[i + 1];
    if (center) this.top = a - (this.height - (b - a)) / 2;
    else if (a - margin < this.top) this.top = a - margin;
    else if (b + margin > this.top + this.height) this.top = b + margin - this.height;
    this.clamp();
  }

  // The element: `render(i)` builds item i. Wheel, size and the thumb are
  // handled here; `cx.notify()` redraws.
  view(t, render) {
    if (this.pending && this.height) {
      const p = this.pending;
      this.pending = null;
      this.apply(p);
    }
    const n = this.sizes?.length ?? 0;
    let start = Math.max(0, this.indexAt(0));
    let end = start;
    while (end < n && this.offsets[end] < this.top + this.height + 1) end++;
    if (!this.height) end = Math.min(n, 60);
    const items = [];
    for (let i = start; i < end; i++) items.push(render(i));
    const total = this.total();
    const over = total > this.height + 1 && this.height > 0;
    const thumb = over ? Math.max(24, (this.height * this.height) / total) : 0;
    const thumbTop = over ? ((this.height - thumb) * this.top) / (total - this.height) : 0;
    const range = this.range();
    const overX = this.horizontal && range > this.vw + 1 && this.vw > 0;
    const thumbX = overX ? Math.max(24, (this.vw * this.vw) / range) : 0;
    const thumbLeft = overX ? ((this.vw - thumbX) * this.left) / (range - this.vw) : 0;
    return div()
      .id(this.id)
      .relative()
      .flex_1()
      .min_h(0)
      .w_full()
      .overflow_hidden()
      .on_size_change((size, cx) => {
        const height = unzoom(size.height);
        const width = unzoom(size.width);
        if (height === this.height && !(this.horizontal && width !== this.vw)) return;
        this.height = height;
        this.vw = width;
        this.clamp();
        this.clampX();
        cx.notify();
      })
      .on_scroll_wheel((e, cx) => {
        const beforeTop = this.top;
        const beforeLeft = this.left;
        const dy = unzoom(e.delta.y);
        // Sideways: a trackpad's delta.x, or shift+wheel (a mouse's vertical
        // wheel turned sideways). Everything else moves vertically.
        if (this.horizontal && e.modifiers?.shift) {
          this.left -= dy;
        } else {
          this.top -= dy;
          if (this.horizontal) this.left -= unzoom(e.delta.x ?? 0);
        }
        this.clamp();
        this.clampX();
        if (this.top !== beforeTop || this.left !== beforeLeft) cx.notify();
      })
      .on_mouse_move((e, cx) => {
        if (!this.dragging) return;
        const d = this.dragging;
        if (d.axis === "x") {
          this.left = d.left + (unzoom(e.position.x - d.x) * (range - this.vw)) / Math.max(1, this.vw - thumbX);
          this.clampX();
        } else {
          this.top = d.top + (unzoom(e.position.y - d.y) * (total - this.height)) / Math.max(1, this.height - thumb);
          this.clamp();
        }
        cx.notify();
      })
      .on_mouse_up("left", () => { this.dragging = null; })
      .child(v_flex().absolute().left(0).right(0).top(this.offsets[start] - this.top).children(items))
      .when(over, (e) =>
        e.child(
          div()
            .id(`${this.id}-thumb`)
            .absolute()
            .right(1)
            .w(BAR)
            .top(thumbTop)
            .h(thumb)
            .rounded(3)
            .bg(t.border)
            .hover((s) => s.bg(t.input ?? t.muted))
            .on_mouse_down("left", (ev, cx) => {
              this.dragging = { axis: "y", y: ev.position.y, top: this.top };
              cx.stop_propagation();
            }),
        ),
      )
      .when(overX, (e) =>
        e.child(
          div()
            .id(`${this.id}-thumb-x`)
            .absolute()
            .bottom(1)
            .h(BAR)
            .left(thumbLeft)
            .w(thumbX)
            .rounded(3)
            .bg(t.border)
            .hover((s) => s.bg(t.input ?? t.muted))
            .on_mouse_down("left", (ev, cx) => {
              this.dragging = { axis: "x", x: ev.position.x, left: this.left };
              cx.stop_propagation();
            }),
        ),
      );
  }
}
