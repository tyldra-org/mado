// Carries requests from the page-world shims (`notifications`,
// `geolocation`, `protocol-handlers`) to the core, and answers back, as DOM
// events. A page can send the same events itself; they only ask for what
// the page could ask for through the real API. Mado world, document start,
// main frame.
import { bridge } from "./lib/bridge";

const b = bridge();
const SCRIPTS: Record<string, string> = {
  notificationRequest: "notifications",
  notificationShow: "notifications",
  notificationClose: "notifications",
  geolocationGet: "geolocation",
  geolocationWatch: "geolocation",
  geolocationClear: "geolocation",
  protocolHandlerRegister: "protocol-handlers",
};

document.addEventListener("mado:request", (e) => {
  const detail = (e as CustomEvent).detail;
  if (!detail || typeof detail !== "object") return;
  const { type, payload } = detail as { type?: unknown; payload?: unknown };
  if (typeof type !== "string") return;
  const script = SCRIPTS[type];
  if (script) b?.send(script, type, payload).catch(() => {});
});

for (const type of ["notificationPermission", "notificationClick", "geolocationPosition"]) {
  b?.on(type, (m) => {
    document.dispatchEvent(new CustomEvent("mado:reply", { detail: JSON.stringify(m) }));
  });
}
