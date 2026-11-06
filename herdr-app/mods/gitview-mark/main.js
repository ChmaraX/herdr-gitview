// "gitview: mark (something's wrong)", from the command palette: the person
// points at a moment. Writes a `mark` to the trace with a snapshot (the
// focused tab's panes, the shared gitview records' sizes and the tab's own,
// what the app says is open), and every gitview copy adds its own state
// within a second (telemetry.js). Draws nothing.

import { View, div } from "gpui-kit";
import { notify, setShared } from "herdr";
import { appCli } from "./engine.js";
import { oneShot } from "./modkit.js"; // links to ../gitview (install copies them)
import { tracing } from "./telemetry.js";

export default class GitviewMark extends View {
  init(_props, cx) {
    oneShot(cx, async (_cx, tr) => {
      if (!tracing()) return notify({ title: "gitview", body: "Tracing is off: nothing to mark (gitview: tracing on/off).", level: "warning" });
      const status = await appCli(["status"]);
      tr.state = () => ({ app: status && { space: status.space, tab: status.tab, overlays: status.overlays ?? [], focused_pane: status.focused_pane ?? null } });
      const id = Date.now();
      setShared("gitview.mark", { id });
      await tr.mark(id);
      notify({ title: "gitview", body: "gitview: marked, tell the lead", level: "info" });
    }, "gitview-mark");
  }

  render() {
    return div();
  }
}
