// App zoom, in one place. The host's on_size_change reports sizes in zoomed
// px (layout px x the app zoom), while .w()/.h()/.top()/.left() and the
// scroller's offsets all take unzoomed layout px, which the app scales
// again. So the mod keeps whatever on_size_change gave it (rawWidth/
// rawHeight, gitview.paneWidth) and converts to layout px here, at the point
// of use — a zoom change resizes no element, so a value converted on arrival
// would go stale. At zoom 1 this is exact identity. Shared by the gitview
// mods (linked; install copies it).

import { settings } from "herdr";

// The app zoom (settings.json "zoom"), 1 when missing, zero or junk — never
// 0 or NaN, so unzoom() can't divide by zero or collapse a list to one row.
export function zoom() {
  return Number(settings()?.zoom) || 1;
}

// A zoomed px measurement (from on_size_change, a wheel/pointer delta or
// gitview.paneWidth) back to the layout px the layout APIs expect.
export function unzoom(px) {
  return px / zoom();
}
