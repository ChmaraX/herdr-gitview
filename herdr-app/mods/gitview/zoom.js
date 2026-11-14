// App zoom. The host measures in zoomed px (layout px x the app zoom): sizes
// from on_size_change, pointer positions, wheel deltas. .w()/.h()/.top()/
// .left() and the scroller's offsets take layout px, which the app scales
// again. unzoom() turns a measurement into layout px as it arrives; at zoom 1
// it is exact identity. Shared by the gitview mods (linked; install copies it).

import { settings } from "herdr";

// A zoomed px measurement in layout px. A missing, zero, negative or
// non-finite "zoom" in settings.json counts as 1.
export function unzoom(px) {
  const z = Number(settings().zoom);
  return Number.isFinite(z) && z > 0 ? px / z : px;
}
