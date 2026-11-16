// `forms`: unsaved-input tracking (spec 04 §7, §9), OpenSearch discovery
// (spec 05 §3) and form detection for Mado Passwords (spec 11 §3). Mado
// world, document end, all frames.
import { sender } from "./lib/bridge";
import { detect, type Field } from "./lib/formdetect";

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

// Form detection (spec 11 §3). Keys are random and live only here: the
// page never sees them, and a key names one element for its lifetime.
const keys = new WeakMap<object, string>();
const byKey = new Map<string, WeakRef<Element | Document>>();

const newKey = (): string => {
  const b = new Uint8Array(9);
  crypto.getRandomValues(b);
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
};

const keyOf = (o: Element | Document): string => {
  let k = keys.get(o);
  if (!k) {
    k = newKey();
    keys.set(o, k);
    byKey.set(k, new WeakRef(o));
  }
  return k;
};

/** The visibility rule of spec 11 §3: rendered, at least 8 px, opaque
 * enough, and not covered at its centre. */
function visible(el: Field): boolean {
  const style = getComputedStyle(el);
  if (style.visibility === "hidden" || style.display === "none") return false;
  if (el.offsetParent === null && style.position !== "fixed") return false;
  const r = el.getBoundingClientRect();
  if (r.width < 8 || r.height < 8) return false;
  for (let e: Element | null = el; e; e = e.parentElement) {
    if (Number.parseFloat(getComputedStyle(e).opacity) <= 0.1) return false;
  }
  const doc = el.ownerDocument;
  const scrollW = doc.documentElement.scrollWidth;
  const scrollH = doc.documentElement.scrollHeight;
  const x = r.left + scrollX;
  const y = r.top + scrollY;
  if (x + r.width <= 0 || y + r.height <= 0 || x >= scrollW || y >= scrollH) return false;
  const cx = r.left + r.width / 2;
  const cy = r.top + r.height / 2;
  if (cx >= 0 && cy >= 0 && cx < innerWidth && cy < innerHeight) {
    const top = doc.elementFromPoint(cx, cy);
    if (
      top &&
      top !== el &&
      !el.contains(top) &&
      !(el.labels && [...el.labels].some((l) => l.contains(top)))
    ) {
      return false;
    }
  }
  return true;
}

let last = "";

function report(): void {
  const forms = detect(document)
    .slice(0, 20)
    .map((f) => ({
      formKey: keyOf(f.root),
      kind: f.kind,
      action: f.root instanceof HTMLFormElement ? f.root.action.slice(0, 2048) || null : null,
      fields: f.fields.slice(0, 50).map(({ el, cls }) => {
        const r = el.getBoundingClientRect();
        return {
          fieldKey: keyOf(el),
          class: cls,
          rect: { x: r.left, y: r.top, width: r.width, height: r.height },
          visible: visible(el),
          inViewport: r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth,
        };
      }),
    }));
  const signature = JSON.stringify(
    forms.map((f) => [f.formKey, f.kind, f.fields.map((x) => [x.fieldKey, x.class, x.visible])]),
  );
  if (signature === last) return;
  last = signature;
  send("forms", { forms });
}

let queued: ReturnType<typeof setTimeout> | undefined;
const later = () => {
  clearTimeout(queued);
  queued = setTimeout(report, 200);
};
new MutationObserver(later).observe(document.documentElement, {
  childList: true,
  subtree: true,
  attributes: true,
  attributeFilter: ["type", "autocomplete", "hidden", "style", "class", "disabled"],
});
report();
