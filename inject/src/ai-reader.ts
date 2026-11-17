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
};

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

if (!g[KEY]) {
  const ai: Ai = { text };
  g[KEY] = ai;
}
