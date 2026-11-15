// `apple-pay` (spec 07 §9): Apple Pay does not exist in WKWebView for a
// self-signed app. A checkout that asks whether it can use Apple Pay hears
// no, and Mado offers to open the page in Safari. Page world, document
// start, main frame.

if (!("ApplePaySession" in window)) {
  let told = false;
  const probe = () => {
    if (told) return;
    told = true;
    document.dispatchEvent(
      new CustomEvent("mado:request", { detail: { type: "applePayProbe", payload: {} } }),
    );
  };
  class ApplePaySession extends EventTarget {
    static canMakePayments(): boolean {
      probe();
      return false;
    }
    static canMakePaymentsWithActiveCard(_merchant: string): Promise<boolean> {
      probe();
      return Promise.resolve(false);
    }
    static supportsVersion(_version: number): boolean {
      return false;
    }
    constructor() {
      super();
      throw new DOMException("Apple Pay is not available", "InvalidAccessError");
    }
  }
  Object.defineProperty(window, "ApplePaySession", {
    value: ApplePaySession,
    writable: true,
    configurable: true,
  });
}

export {};
