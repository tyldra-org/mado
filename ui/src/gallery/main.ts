// The gallery (M6): every screen with fixture state, for design work and
// the screenshot tests. ?screen=window|settings|overlay|internal
// &theme=light|dark &layout=vertical|horizontal &density=comfortable|compact
// &page=<internal page> &section=<settings section> &prompt=<kind>.
import "../design/tokens.css";
import "../design/base.css";
import { type Component, mount } from "svelte";
import { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import { followTheme } from "../design/theme.svelte";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import Internal from "../placeholder/Internal.svelte";
import MainWindow from "../placeholder/MainWindow.svelte";
import SettingsWindow from "../placeholder/SettingsWindow.svelte";
import { answers } from "./answers";
import { state, W } from "./fixtures";
import { prompts } from "./prompts";

const q = new URLSearchParams(location.search);
const screen = q.get("screen") ?? "window";
const theme = q.get("theme") ?? "light";
const prompt = q.get("prompt");
const fixture = state({
  theme,
  layout: q.get("layout") ?? "vertical",
  density: q.get("density") ?? "comfortable",
  prompts: prompt ? prompts(prompt) : {},
});

// The native page view stands in as a card where the UI reports it.
function placePage(rect: { x: number; y: number; width: number; height: number }) {
  let page = document.getElementById("gallery-page");
  if (!page) {
    page = document.createElement("iframe");
    page.id = "gallery-page";
    page.setAttribute("src", "/src/gallery/page.html");
    document.body.append(page);
  }
  Object.assign(page.style, {
    position: "fixed",
    left: `${rect.x}px`,
    top: `${rect.y}px`,
    width: `${rect.width}px`,
    height: `${rect.height}px`,
    border: "0",
    borderRadius: "var(--radius-card)",
    background: "white",
    zIndex: "0",
  });
}

const label = screen === "window" ? `ui-${W}` : screen === "internal" ? "internal" : screen;
const transport: Transport = {
  label: () => label,
  listen: async () => () => {},
  invoke: async (request: Request): Promise<Response> => {
    const cmd = request.cmd as {
      type: string;
      rect?: { x: number; y: number; width: number; height: number };
    };
    if (cmd.type === "state.get") {
      return { status: "ok", id: request.id, value: { rev: 1, state: structuredClone(fixture) } };
    }
    if (cmd.type === "window.reportContentRect" && cmd.rect) placePage(cmd.rect);
    return { status: "ok", id: request.id, value: answers(cmd.type) };
  },
};

const store = new CoreStore(transport);
document.documentElement.dataset.surface = screen === "window" ? "ui" : screen;
followTheme(store);
const target = document.getElementById("app") as HTMLElement;
if (screen === "settings" && q.get("section")) location.hash = q.get("section") ?? "";
const views: Record<string, [Component<never>, Record<string, unknown>]> = {
  window: [MainWindow as Component<never>, { store, windowId: W }],
  settings: [SettingsWindow as Component<never>, { store }],
  internal: [Internal as Component<never>, { store, page: q.get("page") ?? "newtab" }],
};
const [component, props] =
  views[screen] ?? (views.window as [Component<never>, Record<string, unknown>]);
await store.start();
mount(component, { target, props: props as never });
