// `reader` (spec 08 §10): Mozilla Readability on a clone of the page.
// Mado world, on demand. The answer is `{ title, byline, content, length,
// siteName }` or null; the core sanitises `content` before showing it.
import { Readability } from "@mozilla/readability";

const article = (() => {
  try {
    const parsed = new Readability(document.cloneNode(true) as Document).parse();
    if (!parsed?.content) return null;
    return {
      title: parsed.title ?? document.title,
      byline: parsed.byline ?? null,
      content: parsed.content,
      length: parsed.length ?? 0,
      siteName: parsed.siteName ?? null,
    };
  } catch {
    return null;
  }
})();

(globalThis as Record<symbol, unknown>)[Symbol.for("mado.readerResult")] = article;
