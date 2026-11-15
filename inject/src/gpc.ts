// `gpc` (spec 08 §8): Global Privacy Control. WebKit neither sends
// `Sec-GPC` nor defines `navigator.globalPrivacyControl`, so Mado defines
// the property when `privacy.gpc` is on. Page world, document start, every
// frame.

Object.defineProperty(Navigator.prototype, "globalPrivacyControl", {
  get: () => true,
  configurable: true,
  enumerable: true,
});

export {};
