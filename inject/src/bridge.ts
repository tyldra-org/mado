// The message channel between page scripts and the Mado core
// (mado-docs/spec/02-engine.md section 5.2). Runs first, at document start,
// in the Mado content world, in every frame.
//
// It captures WebKit's `mado` message handler and the built-ins it needs
// before any page code runs, and keeps them in this closure. Other Mado-world
// scripts reach it through a frozen object under a registered symbol, which
// the page world (a separate realm) cannot see. The core talks back through
// the inbox function under `Symbol.for("mado.inbox")`, also in this world
// only.

declare const webkit: {
  messageHandlers: Record<string, { postMessage(message: unknown): Promise<unknown> }>;
};

/** Replaced per tab and per document by the adapter when it injects the script. */
const NONCE = "__MADO_NONCE__";

type Send = (script: string, type: string, payload: unknown) => Promise<unknown>;
type Listener = (message: Record<string, unknown>) => void;

const freeze = Object.freeze;
const define = Object.defineProperty;
const handler = webkit.messageHandlers.mado;
// Bound now, so a page that later replaces postMessage on the prototype
// cannot intercept it.
const post = handler ? handler.postMessage.bind(handler) : null;
const listeners = new Map<string, Listener[]>();

const send: Send = (script, type, payload) => {
  if (!post) return Promise.reject(new Error("no Mado channel in this frame"));
  return post(freeze({ v: 1, nonce: NONCE, script, type, payload }));
};

/** Another Mado-world script asks for messages of `type` from the core. */
const on = (type: string, listener: Listener) => {
  const list = listeners.get(type) ?? [];
  list.push(listener);
  listeners.set(type, list);
};

const inbox = (message: unknown) => {
  if (!message || typeof message !== "object") return;
  const m = message as Record<string, unknown>;
  const type = typeof m.type === "string" ? m.type : "";
  for (const listener of listeners.get(type) ?? []) {
    try {
      listener(m);
    } catch {
      // One script's failure must not stop the others.
    }
  }
};

define(globalThis, Symbol.for("mado.bridge"), {
  value: freeze({ send: freeze(send), on: freeze(on) }),
  enumerable: false,
  configurable: false,
  writable: false,
});
define(globalThis, Symbol.for("mado.inbox"), {
  value: freeze(inbox),
  enumerable: false,
  configurable: false,
  writable: false,
});

export {};
