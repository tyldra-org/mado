// Types for the vendored Readability-readerable.js (from @mozilla/readability 0.6.0).
/**
 * Decides whether or not the document is reader-able without parsing the whole thing.
 * @return {boolean} Whether or not we suspect Readability.parse() will suceeed at returning an article object.
 */
declare function isProbablyReaderable(
  document: Document,
  options?: {
    /** The minimum node content length used to decide if the document is readerable. */
    minContentLength?: number;
    /** The minumum cumulated 'score' used to determine if the document is readerable. */
    minScore?: number;
    /** The function used to determine if a node is visible. */
    visibilityChecker?: (node: Node) => boolean;
  },
): boolean;

export = isProbablyReaderable;
