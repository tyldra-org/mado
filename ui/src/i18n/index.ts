import { IntlMessageFormat } from "intl-messageformat";
import en from "./en.json";

// Every user-facing string is a key in en.json, written in ICU MessageFormat
// (mado-docs/spec/00-overview.md, conventions). Other languages come later.
const messages: Record<string, string> = en;
const cache = new Map<string, IntlMessageFormat>();

export function t(key: string, args?: Record<string, string | number>): string {
  const source = messages[key];
  if (source === undefined) {
    return key;
  }
  let format = cache.get(key);
  if (!format) {
    format = new IntlMessageFormat(source, "en");
    cache.set(key, format);
  }
  return String(format.format(args));
}
