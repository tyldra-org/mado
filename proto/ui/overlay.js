// The overlay fixture records the clicks it receives, so experiment 1 can
// tell which layer a click reached.
window.__clicks = [];
addEventListener(
  "click",
  (e) => window.__clicks.push({ x: e.clientX, y: e.clientY, trusted: e.isTrusted }),
  true,
);
