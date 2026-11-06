// "gitview: tracing on/off", from the command palette: shared
// "gitview.trace" (on unless false; telemetry.js). Draws nothing.

import { View, div } from "gpui-kit";
import { notify, setShared } from "herdr";
import { oneShot } from "./modkit.js"; // links to ../gitview (install copies them)
import { traceDir, tracing } from "./telemetry.js";

export default class GitviewTracing extends View {
  init(_props, cx) {
    oneShot(cx, async (_cx, tr) => {
      const on = !tracing();
      if (!on) tr.line("state", { tracing: false }); // the last line, before it stops
      setShared("gitview.trace", on);
      if (on) tr.line("state", { tracing: true });
      notify({ title: "gitview", body: on ? `Tracing on: ${traceDir()}` : "Tracing off", level: "info" });
    }, "gitview-tracing");
  }

  render() {
    return div();
  }
}
