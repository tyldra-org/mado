// `forms`: unsaved-input tracking (spec 04 §7, §9) and OpenSearch discovery
// (spec 05 §3); the rest of forms arrives with Mado Passwords in M4. Mado
// world, document end, all frames.
import { sender } from "./lib/bridge";

const send = sender("forms");
let dirty = false;

const set = (value: boolean) => {
  if (value === dirty) return;
  dirty = value;
  send("unsavedInput", { dirty });
};

const counts = (el: EventTarget | null): boolean => {
  if (el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLInputElement) {
    const type = el.type.toLowerCase();
    return (
      ["text", "email", "url", "tel", "number", ""].includes(type) &&
      el.getAttribute("role") !== "searchbox"
    );
  }
  return el instanceof HTMLElement && el.isContentEditable;
};

document.addEventListener(
  "input",
  (e) => {
    if (counts(e.target)) set(true);
  },
  { capture: true, passive: true },
);
/** The page's OpenSearch description, as an absolute https URL. */
const openSearch = (): string | null => {
  const link = document.querySelector<HTMLLinkElement>(
    'link[rel="search"][type="application/opensearchdescription+xml"]',
  );
  const href = link?.href ?? "";
  return href.startsWith("https://") ? href : null;
};

document.addEventListener(
  "submit",
  (e) => {
    set(false);
    // A search form in the top frame offers the site's engine.
    const form = e.target;
    if (window !== window.top || !(form instanceof HTMLFormElement)) return;
    const searchy =
      form.getAttribute("role") === "search" ||
      form.querySelector('input[type="search"], input[name="q"], input[name="query"]') !== null;
    const href = openSearch();
    if (searchy && href) send("openSearch", { href });
  },
  { capture: true, passive: true },
);
addEventListener("pagehide", () => set(false));
