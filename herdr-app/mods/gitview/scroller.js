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
  // leaves it off and is unchanged). The sideways range is the content width
  // the owner sets (setContent), from the data, not measured from the drawn
  // rows; it resets on another file (resetX). Mutators (setContent, scrollBy,
  // resetX) return whether something changed; the caller redraws.
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
    this.content = 0; // the content's width, in layout px (the owner sets it)
  }

  // The sideways range: the content width, never under the viewport (so a
  // file that fits never scrolls).
  range() {
    return Math.max(this.content, this.vw);
  }

  clampX() {
    this.left = Math.max(0, Math.min(this.left, this.range() - this.vw));
  }

  // The content's width (the widest line plus its room, in layout px), from
  // the data. Returns true when the range or the clamped offset changed.
  setContent(px) {
    if (px === this.content) return false;
    this.content = px;
    this.clampX();
    return true;
  }

  // Sideways by `px` (negative: left); returns true when it moved.
  scrollBy(px) {
    const before = this.left;
    this.left += px;
    this.clampX();
    return this.left !== before;
  }

  // Another file: start sideways from 0 with a fresh range. Returns true
  // when something moved (so the caller can redraw).
  resetX() {
    const changed = this.left !== 0 || this.content !== 0;
    this.left = 0;
    this.content = 0;
    return changed;
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

  // One scrollbar thumb: the same look on either axis, the axis setting which
  // edge it hugs and which way it slides. `start(ev)` is the drag it begins.
  thumb(t, axis, pos, size, start) {
    const e = div()
      .id(`${this.id}-thumb${axis === "x" ? "-x" : ""}`)
      .absolute()
      .rounded(3)
      .bg(t.border)
      .hover((s) => s.bg(t.input ?? t.muted))
      .on_mouse_down("left", (ev, cx) => { this.dragging = start(ev); cx.stop_propagation(); });
    return axis === "x" ? e.bottom(1).left(pos).h(BAR).w(size) : e.right(1).top(pos).w(BAR).h(size);
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
        const changed = height !== this.height || (this.horizontal && width !== this.vw);
        if (!changed) return;
        this.height = height;
        this.vw = width;
        this.clamp();
        this.clampX();
        cx.notify();
      })
      .on_scroll_wheel((e, cx) => {
        const beforeTop = this.top;
        const beforeLeft = this.left;
        const dx = unzoom(e.delta.x ?? 0);
        const dy = unzoom(e.delta.y);
        if (!this.horizontal) {
          this.top -= dy;
        } else if (e.modifiers?.shift && dx === 0) {
          // A mouse wheel turned with shift, on a host that leaves the amount
          // on delta.y (macOS moves it to delta.x; this covers the rest).
          this.left -= dy;
        } else if (Math.abs(dx) > Math.abs(dy)) {
          this.left -= dx; // sideways dominates (a sideways swipe, or a swapped shift+wheel)
        } else {
          this.top -= dy; // vertical dominates: axis-locked, so a straight swipe doesn't drift sideways
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
      .when(over, (e) => e.child(this.thumb(t, "y", thumbTop, thumb, (ev) => ({ axis: "y", y: ev.position.y, top: this.top }))))
      .when(overX, (e) => e.child(this.thumb(t, "x", thumbLeft, thumbX, (ev) => ({ axis: "x", x: ev.position.x, left: this.left }))));
  }
}
