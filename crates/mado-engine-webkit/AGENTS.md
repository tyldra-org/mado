# mado-engine-webkit

The macOS engine adapter: WKWebView, WKWebExtension, content rule lists, downloads. Spec: `mado-docs/spec/02-engine.md` section 4, and section 5.1 of `09-extensions.md`. Root rules in [../../AGENTS.md](../../AGENTS.md) apply.

## Rules

- **Main thread only.** Every WebKit and AppKit object is created, used and dropped on the main thread. Use `MainThreadMarker` from objc2 to make that a compile-time fact.
- **Bindings.** Use `objc2`, `objc2-foundation`, `objc2-app-kit` and `objc2-web-kit`. Declare delegate classes with `define_class!`. No hand-written `msg_send!` where a typed binding exists.
- **`unsafe`.** Allowed here. Every block has a `// SAFETY:` comment naming the invariant (thread, lifetime, nullability).
- **Confirm every selector against the SDK.** Read `WebKit.framework/Headers/` of the installed macOS SDK before using an API, check its availability against macOS 15.4, and record anything the spec got wrong in the milestone's "Spec issues".
- **Private APIs.** Only where the spec names one and says to check `respondsToSelector:` first, with a fallback when it is missing.
- **Map, do not decide.** The adapter turns WebKit callbacks into `EngineEvent`s and `TabCommand`s into WebKit calls. Policy (allow, block, prompt) is decided in the core.
- **Answer every reply.** Each `Reply<T>` from the core times out to the safe default in the spec; a delegate completion handler is always called exactly once.

## Testing

This crate builds only on macOS. Fixture pages go in `tests/pages/` (created with the first engine task) and exercise each event; run them on a real Mac.
