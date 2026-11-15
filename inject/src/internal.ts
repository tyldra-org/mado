// `internal` (spec 01 §4.4): the channel of Mado's own `mado://` pages,
// which run in the page world and have no bridge. It carries their
// `mado:invoke` events to the core and the core's answers and state patches
// back as `mado:invoke-reply` and `mado:patch` events, as JSON strings,
// which cross worlds. On any other page it does nothing; the core also
// refuses `invoke` from any frame that is not a `mado://` page's main frame.
// Mado world, document start, main frame.
import { bridge } from "./lib/bridge";

const b = bridge();

if (location.protocol === "mado:" && b) {
  document.addEventListener("mado:invoke", (e) => {
    const detail = (e as CustomEvent).detail;
    if (typeof detail !== "string") return;
    let parsed: unknown;
    try {
      parsed = JSON.parse(detail);
    } catch {
      return;
    }
    const { id, command } = (parsed ?? {}) as { id?: unknown; command?: unknown };
    if (typeof id !== "number" || !command || typeof command !== "object") return;
    b.send("internal", "invoke", { id, command }).catch(() => {});
  });
  for (const [type, event] of [
    ["invokeReply", "mado:invoke-reply"],
    ["patch", "mado:patch"],
  ] as const) {
    b.on(type, (m) => {
      document.dispatchEvent(new CustomEvent(event, { detail: JSON.stringify(m) }));
    });
  }
}
