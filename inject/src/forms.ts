// `forms`: unsaved-input tracking (spec 04 §7, §9), OpenSearch discovery
// (spec 05 §3) and form detection for Mado Passwords (spec 11 §3). Mado
// world, document end, all frames.
import { bridge, sender } from "./lib/bridge";
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

// The fill menu (spec 11 §4.1): a trusted focus or mouse-down on a
// detected, visible field reports where it is.
const fieldInfo = new WeakMap<Element, { formKey: string }>();
const indexFields = () => {
  for (const f of detect(document)) {
    const formKey = keyOf(f.root);
    for (const { el } of f.fields) fieldInfo.set(el, { formKey });
  }
};
const focused = (e: Event) => {
  if (!e.isTrusted) return;
  const el = e.target;
  if (!(el instanceof HTMLInputElement || el instanceof HTMLSelectElement)) return;
  if (!fieldInfo.has(el)) indexFields();
  const info = fieldInfo.get(el);
  if (!info || !visible(el)) return;
  const r = el.getBoundingClientRect();
  send("fieldFocused", {
    formKey: info.formKey,
    fieldKey: keyOf(el),
    rect: { x: r.left, y: r.top, width: r.width, height: r.height },
  });
};
document.addEventListener("focusin", focused, { capture: true, passive: true });
document.addEventListener("mousedown", focused, { capture: true, passive: true });

// 11 §4.2 step 4: values set with the native setters and announced with
// input and change; never into a field that is gone or hidden.
const inputSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
const selectSetter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")?.set;
const InputEvent = Event;

function setValue(el: Element, value: string): boolean {
  if (el instanceof HTMLSelectElement) {
    const want = value.toLowerCase();
    const option = [...el.options].find(
      (o) => o.value.toLowerCase() === want || o.text.trim().toLowerCase() === want,
    );
    if (!option || !selectSetter) return false;
    selectSetter.call(el, option.value);
  } else if (el instanceof HTMLInputElement && inputSetter) {
    inputSetter.call(el, value);
  } else {
    return false;
  }
  el.dispatchEvent(new InputEvent("input", { bubbles: true }));
  el.dispatchEvent(new InputEvent("change", { bubbles: true }));
  return true;
}

bridge()?.on("fill", (m) => {
  const formKey = typeof m.formKey === "string" ? m.formKey : "";
  const values = (m.values ?? {}) as Record<string, unknown>;
  let filled = 0;
  for (const [fieldKey, value] of Object.entries(values)) {
    const el = byKey.get(fieldKey)?.deref();
    if (!(el instanceof HTMLInputElement || el instanceof HTMLSelectElement)) continue;
    if (typeof value !== "string" || !el.isConnected || !visible(el)) continue;
    if (setValue(el, value)) filled += 1;
  }
  send("fillDone", { formKey, filled });
});
