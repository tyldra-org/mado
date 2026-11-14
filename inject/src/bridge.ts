// The message channel from page scripts to the Mado core
// (mado-docs/spec/02-engine.md section 5.2). Runs first, at document start,
// in the Mado content world, in every frame.
//
// It captures WebKit's `mado` message handler and the built-ins it needs
// before any page code runs, and keeps them in this closure. Other Mado-world
// scripts reach it through a frozen object under a registered symbol, which
// the page world (a separate realm) cannot see.

declare const webkit: {
  messageHandlers: Record<string, { postMessage(message: unknown): Promise<unknown> }>;
};

/** Replaced per tab and per navigation by the core when it injects the script. */
const NONCE = "__MADO_NONCE__";

type Send = (script: string, type: string, payload: unknown) => Promise<unknown>;

const freeze = Object.freeze;
const define = Object.defineProperty;
const handler = webkit.messageHandlers.mado;
// Bound now, so a page that later replaces postMessage on the prototype
// cannot intercept it.
const post = handler ? handler.postMessage.bind(handler) : null;

const send: Send = (script, type, payload) => {
  if (!post) return Promise.reject(new Error("no Mado channel in this frame"));
  return post(freeze({ v: 1, nonce: NONCE, script, type, payload }));
};

define(globalThis, Symbol.for("mado.bridge"), {
  value: freeze({ send: freeze(send) }),
  enumerable: false,
  configurable: false,
  writable: false,
});
