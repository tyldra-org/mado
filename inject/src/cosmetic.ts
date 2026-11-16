// `cosmetic` (spec 10 §6, §8): hides what the blocker's cosmetic filters
// name, runs its procedural filters, asks the core for generic hiding rules
// for the class and id names it sees, and hosts the element picker. Mado
// world, document start, every frame. The data for this document comes
// from the navigation's own script as globalThis[Symbol.for("mado.cosmetic")].

import { bridge, sender } from "./lib/bridge";

type Step = { type: string; arg?: unknown };
type Procedural = { selector: Step[]; action?: { type: string; arg?: string } | null };
type Data = { hide: string[]; procedural: string[]; generic: boolean };

const send = sender("cosmetic");
const data = (globalThis as Record<symbol, unknown>)[Symbol.for("mado.cosmetic")] as
  | Data
  | undefined;
const STYLE_ID = "mado-cosmetic";
const hidden = new Set<string>();

/** Batches style rules into one stylesheet the page cannot see change. */
function hide(selectors: string[]): void {
  const fresh = selectors.filter((s) => s && !hidden.has(s));
  if (fresh.length === 0) return;
  for (const s of fresh) hidden.add(s);
  const root = document.documentElement;
  if (!root) return;
  let style = document.getElementById(STYLE_ID) as HTMLStyleElement | null;
  if (!style) {
    style = document.createElement("style");
    style.id = STYLE_ID;
    (document.head ?? root).append(style);
  }
  // One rule per selector: a selector the browser rejects only loses itself.
  for (const s of fresh) {
    try {
      style.sheet?.insertRule(`${s} { display: none !important; }`, style.sheet.cssRules.length);
    } catch {}
  }
}

function textMatches(el: Element, arg: unknown): boolean {
  const text = el.textContent ?? "";
  if (typeof arg !== "string") return false;
  const m = /^\/(.*)\/([a-z]*)$/.exec(arg);
  if (m) {
    try {
      return new RegExp(m[1] ?? "", m[2]).test(text);
    } catch {
      return false;
    }
  }
  return text.includes(arg);
}

/** The elements a procedural filter's steps select (css-selector, has-text, upward). */
export function select(steps: Step[], root: ParentNode = document): Element[] {
  let current = null as Element[] | null;
  for (const step of steps) {
    if (step.type === "css-selector") {
      const sel = String(step.arg ?? "");
      try {
        current =
          current === null
            ? [...root.querySelectorAll(sel)]
            : current.filter((el) => el.matches(sel));
      } catch {
        return [];
      }
    } else if (step.type === "has-text") {
      current = (current ?? [...root.querySelectorAll("*")]).filter((el) =>
        textMatches(el, step.arg),
      );
    } else if (step.type === "upward") {
      const arg = step.arg;
      const from: Element[] = current ?? [];
      current = from.flatMap((el: Element): Element[] => {
        if (typeof arg === "number" || /^\d+$/.test(String(arg))) {
          let up: Element | null = el;
          for (let i = 0; i < Number(arg) && up; i++) up = up.parentElement;
          return up ? [up] : [];
        }
        const found = el.parentElement?.closest(String(arg));
        return found ? [found] : [];
      });
    } else {
      // An operator Mado does not run yet selects nothing.
      return [];
    }
  }
  return current ?? [];
}

function runProcedural(): void {
  for (const raw of data?.procedural ?? []) {
    let filter: Procedural;
    try {
      filter = JSON.parse(raw) as Procedural;
    } catch {
      continue;
    }
    for (const el of select(filter.selector)) {
      const action = filter.action;
      if (!action) (el as HTMLElement).style?.setProperty("display", "none", "important");
      else if (action.type === "remove") el.remove();
      else if (action.type === "style" && action.arg) el.setAttribute("style", action.arg);
      else if (action.type === "remove-attr" && action.arg) el.removeAttribute(action.arg);
      else if (action.type === "remove-class" && action.arg) el.classList.remove(action.arg);
    }
  }
}

// Generic hiding: class and id names, sent in batches every 300 ms.
const seenClasses = new Set<string>();
const seenIds = new Set<string>();
let pendingClasses: string[] = [];
let pendingIds: string[] = [];
let timer = 0;

function collect(el: Element): void {
  if (el.id && !seenIds.has(el.id)) {
    seenIds.add(el.id);
    pendingIds.push(el.id);
  }
  for (const c of el.classList) {
    if (!seenClasses.has(c)) {
      seenClasses.add(c);
      pendingClasses.push(c);
    }
  }
}

function flush(): void {
  timer = 0;
  if (pendingClasses.length === 0 && pendingIds.length === 0) return;
  send("cosmeticClasses", {
    classes: pendingClasses.slice(0, 1000),
    ids: pendingIds.slice(0, 1000),
  });
  pendingClasses = [];
  pendingIds = [];
}

function scan(nodes: Iterable<Node>): void {
  for (const n of nodes) {
    if (!(n instanceof Element)) continue;
    collect(n);
    for (const el of n.querySelectorAll("[class],[id]")) collect(el);
  }
  if (data?.generic && !timer) timer = window.setTimeout(flush, 300);
}

// The element picker (10 §8).
let picking = false;
let highlight: HTMLDivElement | null = null;
let target: Element | null = null;
let idle = 0;

const GENERATED = /[a-z]+-[0-9a-f]{5,}/i;

/** A selector for `el`: its id, then stable classes, then an nth-of-type path. */
export function selectorFor(el: Element): string {
  if (el.id && !GENERATED.test(el.id)) return `#${CSS.escape(el.id)}`;
  const classes = [...el.classList].filter((c) => !GENERATED.test(c));
  if (classes.length > 0) return `${el.localName}.${classes.map((c) => CSS.escape(c)).join(".")}`;
  const path: string[] = [];
  for (let e: Element | null = el; e && e !== document.documentElement; e = e.parentElement) {
    const parent: Element | null = e.parentElement;
    const same = parent ? [...parent.children].filter((c) => c.localName === e?.localName) : [];
    path.unshift(
      same.length > 1 ? `${e.localName}:nth-of-type(${same.indexOf(e) + 1})` : e.localName,
    );
  }
  return path.join(" > ");
}

function stopPicking(selector: string | null): void {
  if (!picking) return;
  picking = false;
  highlight?.remove();
  highlight = null;
  window.clearTimeout(idle);
  document.removeEventListener("mousemove", onMove, true);
  document.removeEventListener("click", onClick, true);
  document.removeEventListener("keydown", onKey, true);
  send("pickerResult", { selector });
}

function onMove(e: MouseEvent): void {
  const el = document.elementFromPoint(e.clientX, e.clientY);
  if (!el || el === highlight) return;
  target = el;
  const r = el.getBoundingClientRect();
  if (highlight) {
    Object.assign(highlight.style, {
      left: `${r.left}px`,
      top: `${r.top}px`,
      width: `${r.width}px`,
      height: `${r.height}px`,
    });
  }
}

function onClick(e: MouseEvent): void {
  e.preventDefault();
  e.stopImmediatePropagation();
  const at = document.elementFromPoint(e.clientX, e.clientY) ?? target;
  if (at) {
    // A wider choice: Option-click takes the parent.
    const el = e.altKey && at.parentElement ? at.parentElement : at;
    const selector = selectorFor(el);
    hide([selector]);
    stopPicking(selector);
  }
}

function onKey(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    e.preventDefault();
    stopPicking(null);
  }
}

function startPicking(): void {
  if (picking || window !== window.top) return;
  picking = true;
  highlight = document.createElement("div");
  Object.assign(highlight.style, {
    position: "fixed",
    pointerEvents: "none",
    zIndex: "2147483647",
    outline: "2px solid #e5484d",
    background: "rgba(229, 72, 77, 0.15)",
  });
  document.documentElement.append(highlight);
  document.addEventListener("mousemove", onMove, true);
  document.addEventListener("click", onClick, true);
  document.addEventListener("keydown", onKey, true);
  // Two idle minutes cancel.
  idle = window.setTimeout(() => stopPicking(null), 120_000);
}

if (data) {
  hide(data.hide);
  const start = () => {
    runProcedural();
    scan([document.documentElement]);
  };
  if (document.readyState === "loading")
    document.addEventListener("DOMContentLoaded", start, { once: true });
  else start();
  new MutationObserver((records) => {
    for (const r of records) scan(r.addedNodes);
    if (data.procedural.length > 0) runProcedural();
  }).observe(document, { childList: true, subtree: true });
}

bridge()?.on("cosmeticSelectors", (m) => hide((m.selectors as string[] | undefined) ?? []));
bridge()?.on("pickerStart", () => startPicking());
