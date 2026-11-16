// Spec 09 §12 check 6 with a fixture host: sends one message and opens a
// port to app.mado.test_echo, and reports each answer by opening a tab whose
// URL carries it. Retries until the host is registered.
const HOST = "app.mado.test_echo";
const report = (kind, value) =>
  chrome.tabs.create({
    url: `http://pages.mado.test/extensions/plain.html?${kind}=${encodeURIComponent(JSON.stringify(value))}`,
  });

let sent = false;
let connected = false;
const tryNow = () => {
  if (!sent) {
    chrome.runtime
      .sendNativeMessage(HOST, { ping: "once" })
      .then((reply) => {
        if (!sent) {
          sent = true;
          report("once", reply);
        }
      })
      .catch(() => {});
  }
  if (!connected) {
    try {
      const port = chrome.runtime.connectNative(HOST);
      port.onMessage.addListener((m) => {
        if (!connected) {
          connected = true;
          report("port", m);
          port.disconnect();
        }
      });
      port.postMessage({ ping: "port" });
    } catch {}
  }
  if (!sent || !connected) setTimeout(tryNow, 1000);
};
tryNow();
