// `protocol-handlers` (spec 07 §6): `navigator.registerProtocolHandler` on
// WebKit, which lacks it. Page world, document start, main frame.

Object.defineProperty(navigator, "registerProtocolHandler", {
  value: (scheme: string, url: string) => {
    const s = String(scheme).toLowerCase();
    const u = String(url);
    if (
      !/^(web\+[a-z]+|mailto|tel|sms|irc|ircs|magnet|bitcoin|geo|im|matrix|xmpp|webcal)$/.test(s) ||
      !u.includes("%s")
    ) {
      throw new DOMException("Invalid scheme or URL", "SecurityError");
    }
    document.dispatchEvent(
      new CustomEvent("mado:request", {
        detail: {
          type: "protocolHandlerRegister",
          payload: { scheme: s, url: new URL(u, location.href).href.slice(0, 2048) },
        },
      }),
    );
  },
  configurable: true,
});

export {};
