// `status` (spec 02 §5, §5.3): the link under the pointer, the page's
// icons, a heartbeat every 2 seconds, and the page text for history search
// (spec 12 §2.2). Mado world, document start, main frame.
import { sender } from "./lib/bridge";

const send = sender("status");

// Heartbeat: a page stuck in a loop stops sending it.
setInterval(() => send("heartbeat", {}), 2000);
send("heartbeat", {});

let lastHover: string | null = null;
const hover = (url: string | null) => {
  if (url === lastHover) return;
  lastHover = url;
  send("hoverLink", { url });
};
document.addEventListener(
  "mouseover",
  (e) => {
    const a = e.target instanceof Element ? e.target.closest("a[href]") : null;
    hover(a instanceof HTMLAnchorElement ? a.href : null);
  },
  { capture: true, passive: true },
);
document.addEventListener("mouseleave", () => hover(null), { capture: true, passive: true });

const MAX_ICONS = 16;
let lastIcons = "";
function icons() {
  const found = Array.from(
    document.querySelectorAll<HTMLLinkElement>("link[rel~='icon'], link[rel='apple-touch-icon']"),
  )
    .slice(0, MAX_ICONS)
    .map((l) => ({ url: l.href, sizes: l.getAttribute("sizes"), rel: l.rel }))
    .filter((i) => i.url.startsWith("http"));
  if (found.length === 0)
    found.push({ url: new URL("/favicon.ico", location.href).href, sizes: null, rel: "icon" });
  const key = JSON.stringify(found);
  if (key === lastIcons) return;
  lastIcons = key;
  send("favicons", { icons: found });
}

const PAGE_TEXT_LIMIT = 200 * 1024;
function pageText() {
  // Pages with a password or card field are never indexed (spec 12 §2.2).
  if (document.querySelector("input[type=password], input[autocomplete^='cc-']")) return;
  // biome-ignore lint/performance/useDomNodeTextContent: the visible text is what history search needs (spec 12 §2.2).
  const text = (document.body?.innerText ?? "").replace(/\s+/g, " ").trim();
  if (text) send("pageText", { text: text.slice(0, PAGE_TEXT_LIMIT) });
}

addEventListener("DOMContentLoaded", () => {
  icons();
  new MutationObserver(icons).observe(document.head ?? document.documentElement, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ["href", "rel"],
  });
});
addEventListener("load", () => {
  // Three seconds idle after the load (spec 12 §2.2).
  setTimeout(pageText, 3000);
});
