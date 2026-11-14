// `find` (spec 02 §5): counts matches and tracks the active one; WebKit
// highlights. Mado world, document end, main frame.
import { bridge, sender } from "./lib/bridge";

const send = sender("find");
const MAX = 1000;
let query = "";
let caseSensitive = false;
let count = 0;
let active = 0;

function countMatches(): number {
  if (!query || !document.body) return 0;
  const needle = caseSensitive ? query : query.toLowerCase();
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) => {
      const p = n.parentElement;
      if (!p || p.closest("script, style, noscript, template")) return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  let n = 0;
  for (let node = walker.nextNode(); node && n < MAX; node = walker.nextNode()) {
    const text = caseSensitive ? (node.nodeValue ?? "") : (node.nodeValue ?? "").toLowerCase();
    for (
      let i = text.indexOf(needle);
      i >= 0 && n < MAX;
      i = text.indexOf(needle, i + needle.length)
    )
      n++;
  }
  return n;
}

const report = () =>
  send("findResult", { matches: query ? count : null, active: query && count ? active : null });

const b = bridge();
b?.on("find", (m) => {
  query = typeof m.query === "string" ? m.query.slice(0, 1000) : "";
  caseSensitive = m.caseSensitive === true;
  count = countMatches();
  active = count ? 1 : 0;
  report();
});
b?.on("findNext", (m) => {
  if (!count) return report();
  active = m.forward === false ? ((active - 2 + count) % count) + 1 : (active % count) + 1;
  report();
});
b?.on("findClear", () => {
  getSelection()?.removeAllRanges();
  query = "";
  count = 0;
  active = 0;
  report();
});
