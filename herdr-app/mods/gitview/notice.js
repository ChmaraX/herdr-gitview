// What the list's notice() has to act on, as one pure decision (stale()
// gathers the facts; tested in herdr-app/test). The first that holds, in
// this order, or null:
//
// | reason    | when                                    | notice() then                 |
// |-----------|-----------------------------------------|-------------------------------|
// | "panes"   | a mod pane closed since the last look   | revive the lost loads (bug #1)|
// | "panel"   | the panel showed or hid since           | open the file / close the unit|
// | "aside"   | cmd-g's tab came or went                | step aside / come back        |
// | null      | the list isn't on screen                | nothing more (its timer looks)|
// | "handoff" | a handoff not seen yet, or nvim's pane  | followHandoff                 |
// |           | came or went (not every look while open)|                               |
// | "stamp"   | another list's watch stamped the repo   | refresh                       |
// | "place"   | another repo (or scope)                 | refresh                       |
// | "tab"     | another tab or space                    | its pane's file, its handoff  |
// | "ask"     | the pane asked something new            | followAsk                     |
//
// `s`: {panesMoved, cycle: null or {was: the shown it last saw, aside:
// stepAside has something to do}, shown, handoff: null or {seen, nvim:
// nvim's pane is there now, nvimSeen: as last looked}, stamp, key, wasKey,
// tab, wasTab, ask: the live ask's seq (0: none), askSeen}.
export function staleWhy(s) {
  if (s.panesMoved) return "panes";
  if (s.cycle && s.cycle.was !== s.shown) return "panel";
  if (s.cycle?.aside) return "aside";
  if (!s.shown) return null;
  if (s.handoff && (!s.handoff.seen || s.handoff.nvim !== s.handoff.nvimSeen)) return "handoff";
  if (s.stamp) return "stamp";
  if (s.key !== s.wasKey) return "place";
  if (s.tab !== s.wasTab) return "tab";
  if (s.ask > s.askSeen) return "ask";
  return null;
}
