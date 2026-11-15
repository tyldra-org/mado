// Types for the vendored Readability.js (from @mozilla/readability 0.6.0).
declare class Readability<T = string> {
  constructor(
    document: Document,
    options?: {
      debug?: boolean;
      maxElemsToParse?: number;
      nbTopCandidates?: number;
      charThreshold?: number;
      classesToPreserve?: string[];
      keepClasses?: boolean;
      serializer?: (node: Node) => T;
      disableJSONLD?: boolean;
      allowedVideoRegex?: RegExp;
    },
  );

  parse(): null | {
    /** article title */
    title: string | null | undefined;

    /** HTML string of processed article content */
    content: T | null | undefined;

    /** text content of the article, with all the HTML tags removed */
    textContent: string | null | undefined;

    /** length of an article, in characters */
    length: number | null | undefined;

    /** article description, or short excerpt from the content */
    excerpt: string | null | undefined;

    /** author metadata */
    byline: string | null | undefined;

    /** content direction */
    dir: string | null | undefined;

    /** name of the site */
    siteName: string | null | undefined;

    /** content language */
    lang: string | null | undefined;

    /** published time */
    publishedTime: string | null | undefined;
  };
}

export = Readability;
