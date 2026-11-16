// `store` (spec 09 §2.1): on a Chrome Web Store detail page, hides the
// "Switch to Chrome" banner and turns the install button into "Add to
// Mado", "Remove from Mado" or "Not available in Mado". Mado world,
// document end, main frame, chromewebstore.google.com only. The core trusts
// it for nothing but the ID, and tells it what is installed and what is
// unavailable through globalThis[Symbol.for("mado.store")].

import { sender } from "./lib/bridge";

const send = sender("store");

interface Known {
  installed: string[];
  unavailable: string[];
  labels: { add: string; remove: string; unavailable: string };
}

const KNOWN = Symbol.for("mado.store");
const MARK = "data-mado-store";

/** The ID in /detail/<slug>/<id> or /detail/<id>. */
export function idFromPath(path: string): string | null {
  const parts = path.split("/").filter(Boolean);
  if (parts[0] !== "detail") return null;
  const id = parts[parts.length - 1] ?? "";
  return /^[a-p]{32}$/.test(id) ? id : null;
}

/** Whether the page shows a theme: a link to the store's themes category. */
function isTheme(): boolean {
  return document.querySelector('a[href*="/category/themes"]') !== null;
}

/** The detail header's install button: "Add to Chrome" or "Remove from Chrome". */
function installButton(): HTMLButtonElement | null {
  for (const b of document.querySelectorAll<HTMLButtonElement>("button")) {
    const text = b.textContent?.trim() ?? "";
    if (b.hasAttribute(MARK) || /(Add to|Remove from) Chrome/i.test(text)) return b;
  }
  return null;
}

function hideBanner(): void {
  for (const el of document.querySelectorAll<HTMLElement>("[role=alert], [role=status], aside")) {
    if (/Switch to Chrome/i.test(el.textContent ?? "")) el.style.display = "none";
  }
}

function update(): void {
  if (location.host !== "chromewebstore.google.com") return;
  hideBanner();
  const id = idFromPath(location.pathname);
  const known = (globalThis as Record<symbol, unknown>)[KNOWN] as Known | undefined;
  const button = installButton();
  if (!id || !known || !button) return;
  const unavailable = known.unavailable.includes(id) || isTheme();
  const installed = known.installed.includes(id);
  const label = unavailable
    ? known.labels.unavailable
    : installed
      ? known.labels.remove
      : known.labels.add;
  const span = button.querySelector("span") ?? button;
  if (span.textContent !== label) span.textContent = label;
  button.disabled = unavailable;
  if (!button.hasAttribute(MARK)) {
    button.setAttribute(MARK, "");
    button.addEventListener(
      "click",
      (event) => {
        event.preventDefault();
        event.stopImmediatePropagation();
        const now = idFromPath(location.pathname);
        const k = (globalThis as Record<symbol, unknown>)[KNOWN] as Known | undefined;
        if (!now || !k || button.disabled) return;
        send("storeInstall", { id: now, remove: k.installed.includes(now) });
      },
      true,
    );
  }
}

if (location.host === "chromewebstore.google.com") {
  // The store is a single-page app: labels follow every change.
  let queued = false;
  new MutationObserver(() => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      update();
    });
  }).observe(document.documentElement, { childList: true, subtree: true });
  (globalThis as Record<symbol, unknown>)[Symbol.for("mado.storeUpdate")] = update;
  update();
}
