// The prototype's UI: an address field above the page area. It reports the
// page rectangle to the shell, which places the native tab view over it.
const { invoke } = window.__TAURI__.core;
const form = document.querySelector("#address");
const input = document.querySelector("#url");
const page = document.querySelector("#page");
const status = document.querySelector("#status");

form.addEventListener("submit", (event) => {
  event.preventDefault();
  invoke("navigate", { url: input.value }).then(
    () => {
      status.textContent = "";
    },
    (error) => {
      status.textContent = String(error);
    },
  );
});

let frame = 0;
function reportRect() {
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(() => {
    const r = page.getBoundingClientRect();
    const rect = { x: r.x, y: r.y, width: r.width, height: r.height };
    invoke("report_content_rect", { rect }).catch((error) => {
      status.textContent = String(error);
    });
  });
}
new ResizeObserver(reportRect).observe(page);

// Used by `mado-proto --smoke` to type into the field like a user would.
window.madoProto = {
  typeAndGo(url) {
    input.value = url;
    form.requestSubmit();
  },
};
