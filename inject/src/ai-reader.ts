// `ai-reader` (spec 13 §1, §5): the page's readable text for AI. Mado
// world, evaluated on demand only while AI is on, so nothing of it runs
// in a page otherwise. Installs itself once per document; the core then
// calls one of its functions and reads the answer from
// `Symbol.for("mado.aiResult")`.
import Readability from "../vendor/readability/Readability.js";

type Ai = {
  text: () => {
    title: string;
    url: string;
    text: string;
    readable: boolean;
    sensitiveField: boolean;
  };
  snapshot: (opts: { interactive?: boolean; ref?: string | null }) => {
    title: string;
    url: string;
    lines: string[];
    sensitiveField: boolean;
  };
  locate: (ref: string) => Locate;
  focus: (ref: string) => boolean;
  choose: (ref: string, value: string) => string | null;
  scroll: (ref: string | null, dx: number, dy: number) => boolean;
  quiet: () => number;
};

/** Where to click an element, or what covers it (spec 13 §8). */
type Locate =
  | { found: false }
  | { found: true; x: number; y: number; covered: string | null; inView: boolean };

const KEY = Symbol.for("mado.ai");
const g = globalThis as Record<symbol, unknown>;

/// Whether an element takes up room on screen.
function visible(el: Element): boolean {
  const r = el.getBoundingClientRect();
  if (r.width < 1 || r.height < 1) return false;
  const s = getComputedStyle(el);
  return s.visibility !== "hidden" && s.display !== "none" && Number(s.opacity) > 0;
}

/// 13 §9.4: a visible password, card or ID-number field makes the page
/// sensitive.
const SENSITIVE = [
  "input[type=password]",
  "input[autocomplete^='cc-']",
  "input[autocomplete='one-time-code']",
  "input[name*='ssn' i]",
  "input[id*='ssn' i]",
  "input[name*='passport' i]",
  "input[name*='iban' i]",
  "input[name*='cardnumber' i]",
  "input[name*='card_number' i]",
].join(",");

function text(): ReturnType<Ai["text"]> {
  const sensitiveField = Array.from(document.querySelectorAll(SENSITIVE)).some(visible);
  let body = "";
  let readable = false;
  try {
    const parsed = new Readability(document.cloneNode(true) as Document).parse();
    const t = parsed?.textContent?.replace(/\s+/g, " ").trim() ?? "";
    if (t.length > 200) {
      body = t;
      readable = true;
    }
  } catch {
    // Readability failed: the visible text below.
  }
  if (!readable) {
    // biome-ignore lint/performance/useDomNodeTextContent: the visible text is what a reader sees.
    body = (document.body?.innerText ?? "").replace(/\s+/g, " ").trim();
  }
  return { title: document.title, url: location.href, text: body, readable, sensitiveField };
}

// Snapshot refs (13 §8): `e<n>` for elements, kept in a WeakMap-backed
// table so a ref stays valid while its element is attached.
const refs = new Map<string, WeakRef<Element>>();
const ids = new WeakMap<Element, string>();
let nextRef = 1;

function refOf(el: Element): string {
  let r = ids.get(el);
  if (!r) {
    r = `e${nextRef++}`;
    ids.set(el, r);
    refs.set(r, new WeakRef(el));
  }
  return r;
}

function byRef(ref: string): Element | null {
  const el = refs.get(ref)?.deref() ?? null;
  return el?.isConnected ? el : null;
}

const IMPLICIT: Record<string, string> = {
  A: "link",
  BUTTON: "button",
  SELECT: "combobox",
  TEXTAREA: "textbox",
  IMG: "img",
  H1: "heading",
  H2: "heading",
  H3: "heading",
  H4: "heading",
  H5: "heading",
  H6: "heading",
  NAV: "navigation",
  MAIN: "main",
  FORM: "form",
  TABLE: "table",
  LI: "listitem",
  UL: "list",
  OL: "list",
  DIALOG: "dialog",
  LABEL: "label",
  SUMMARY: "button",
  OPTION: "option",
  P: "paragraph",
};

function role(el: Element): string {
  const explicit = el.getAttribute("role");
  if (explicit) return explicit.split(" ")[0] ?? explicit;
  if (el instanceof HTMLInputElement) {
    const t = el.type;
    if (t === "checkbox") return "checkbox";
    if (t === "radio") return "radio";
    if (t === "submit" || t === "button" || t === "reset" || t === "image") return "button";
    if (t === "range") return "slider";
    if (t === "search") return "searchbox";
    return "textbox";
  }
  if (el instanceof HTMLAnchorElement && !el.hasAttribute("href")) return "";
  if ((el as HTMLElement).isContentEditable) return "textbox";
  return IMPLICIT[el.tagName] ?? "";
}

const INTERACTIVE = new Set([
  "link",
  "button",
  "textbox",
  "searchbox",
  "checkbox",
  "radio",
  "combobox",
  "slider",
  "option",
  "menuitem",
  "tab",
  "switch",
]);

function name(el: Element): string {
  const label = el.getAttribute("aria-label");
  if (label) return label;
  const by = el.getAttribute("aria-labelledby");
  if (by) {
    const t = by
      .split(/\s+/)
      .map((id) => document.getElementById(id)?.textContent ?? "")
      .join(" ")
      .trim();
    if (t) return t;
  }
  if (
    el instanceof HTMLInputElement ||
    el instanceof HTMLTextAreaElement ||
    el instanceof HTMLSelectElement
  ) {
    const l = el.labels?.[0]?.textContent?.trim();
    if (l) return l;
    const p = el.getAttribute("placeholder");
    if (p) return p;
    if (el instanceof HTMLInputElement && (el.type === "submit" || el.type === "button"))
      return el.value;
  }
  if (el instanceof HTMLImageElement) return el.alt;
  return (el as HTMLElement).textContent?.replace(/\s+/g, " ").trim().slice(0, 100) ?? "";
}

function states(el: Element): string[] {
  const out: string[] = [];
  if (
    el instanceof HTMLInputElement &&
    (el.type === "checkbox" || el.type === "radio") &&
    el.checked
  )
    out.push("checked");
  if ((el as HTMLInputElement).disabled) out.push("disabled");
  if (el.getAttribute("aria-expanded") === "true") out.push("expanded");
  if (el.getAttribute("aria-selected") === "true") out.push("selected");
  if (document.activeElement === el) out.push("focused");
  if ((el as HTMLInputElement).required) out.push("required");
  return out;
}

function value(el: Element): string | null {
  if (el instanceof HTMLInputElement) {
    if (el.type === "password") return el.value ? "••••" : "";
    if (el.type === "checkbox" || el.type === "radio") return null;
    return el.value;
  }
  if (el instanceof HTMLTextAreaElement) return el.value;
  if (el instanceof HTMLSelectElement) return el.selectedOptions[0]?.textContent?.trim() ?? "";
  return null;
}

const quote = (s: string) => JSON.stringify(s.length > 100 ? `${s.slice(0, 100)}…` : s);

function walk(root: Element, interactive: boolean, depth: number, lines: string[]) {
  if (lines.length > 3000) return;
  for (const el of Array.from(root.children)) {
    if (!visible(el) && !(el instanceof HTMLOptionElement)) {
      // A hidden parent can still hold visible fixed children; skip only
      // what has no box and no children.
      if (el.children.length === 0) continue;
    }
    const r = role(el);
    const shown = r && (!interactive || INTERACTIVE.has(r));
    let nextDepth = depth;
    if (shown) {
      const v = value(el);
      const st = states(el);
      const nm = name(el);
      lines.push(
        `${"  ".repeat(depth)}${refOf(el)} ${r}${nm ? ` ${quote(nm)}` : ""}${v !== null ? ` value=${quote(v)}` : ""}${st.length ? ` [${st.join(", ")}]` : ""}`,
      );
      nextDepth = depth + 1;
      // Leaves: links and buttons need no inside.
      if (INTERACTIVE.has(r) && r !== "combobox") continue;
    } else if (!interactive && el.children.length === 0) {
      const t = (el as HTMLElement).textContent?.replace(/\s+/g, " ").trim();
      if (t && visible(el)) lines.push(`${"  ".repeat(depth)}text ${quote(t)}`);
      continue;
    }
    walk(el, interactive, nextDepth, lines);
  }
}

function snapshot(opts: {
  interactive?: boolean;
  ref?: string | null;
}): ReturnType<Ai["snapshot"]> {
  const root = (opts.ref ? byRef(opts.ref) : document.body) ?? document.body;
  const lines: string[] = [];
  if (root) walk(root, !!opts.interactive, 0, lines);
  const sensitiveField = Array.from(document.querySelectorAll(SENSITIVE)).some(visible);
  return { title: document.title, url: location.href, lines, sensitiveField };
}

function describe(el: Element): string {
  const r = role(el) || el.tagName.toLowerCase();
  const n = name(el);
  return `${refOf(el)} ${r}${n ? ` ${quote(n)}` : ""}`;
}

function locate(ref: string): Locate {
  const el = byRef(ref);
  if (!el) return { found: false };
  el.scrollIntoView({ block: "center", inline: "center" });
  const r = el.getBoundingClientRect();
  const x = r.left + r.width / 2;
  const y = r.top + r.height / 2;
  const inView = x >= 0 && y >= 0 && x <= innerWidth && y <= innerHeight;
  const top = document.elementFromPoint(x, y);
  const covered =
    top && top !== el && !el.contains(top) && !top.contains(el) ? describe(top) : null;
  return { found: true, x, y, covered, inView };
}

function focus(ref: string): boolean {
  const el = byRef(ref) as HTMLElement | null;
  if (!el) return false;
  el.scrollIntoView({ block: "center" });
  el.focus();
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) el.select();
  return document.activeElement === el;
}

function choose(ref: string, wanted: string): string | null {
  const el = byRef(ref);
  if (!(el instanceof HTMLSelectElement)) return null;
  const opt = Array.from(el.options).find(
    (o) => o.value === wanted || o.textContent?.trim() === wanted,
  );
  if (!opt) return null;
  el.value = opt.value;
  el.dispatchEvent(new Event("input", { bubbles: true }));
  el.dispatchEvent(new Event("change", { bubbles: true }));
  return opt.textContent?.trim() ?? opt.value;
}

function scroll(ref: string | null, dx: number, dy: number): boolean {
  if (ref) {
    const el = byRef(ref);
    if (!el) return false;
    if (dx === 0 && dy === 0) el.scrollIntoView({ block: "center" });
    else el.scrollBy(dx, dy);
    return true;
  }
  scrollBy(dx, dy);
  return true;
}

// Settling (13 §8): when the DOM last changed.
// Each evaluation runs this file again; only the first run's functions
// and observer stay, under `Symbol.for("mado.ai")`.
let lastMutation = performance.now();
const quiet = () => performance.now() - lastMutation;

if (!g[KEY]) {
  new MutationObserver(() => {
    lastMutation = performance.now();
  }).observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
  const ai: Ai = { text, snapshot, locate, focus, choose, scroll, quiet };
  g[KEY] = ai;
}
