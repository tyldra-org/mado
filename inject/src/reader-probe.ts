// `reader` probe (spec 08 §10): whether the page looks like an article,
// with Readability's `isProbablyReaderable`. Mado world, on demand after
// the page finishes loading; the result is the evaluation's answer.
import isProbablyReaderable from "../vendor/readability/Readability-readerable.js";

(globalThis as Record<symbol, unknown>)[Symbol.for("mado.readerResult")] =
  document.body !== null && isProbablyReaderable(document);
