// Menu commands the window's UI carries out itself (spec 01 §5.3, §5.4):
// the core sends `mado://action { action }` to the key window's UI.
import type { CoreStore } from "./store.svelte";

export type Action =
  | "openLocation"
  | "focusAddressBar"
  | "find"
  | "findNext"
  | "findPrevious"
  | "bookmarkTab"
  | "bookmarkAllTabs";

/** Calls `handler` for each action; returns a function that stops. */
export function onAction(store: CoreStore, handler: (action: Action) => void): () => void {
  let stop: (() => void) | undefined;
  let stopped = false;
  void store.transport
    .listen<{ action: Action }>("mado://action", (p) => handler(p.action))
    .then((s) => {
      if (stopped) s();
      else stop = s;
    });
  return () => {
    stopped = true;
    stop?.();
  };
}
