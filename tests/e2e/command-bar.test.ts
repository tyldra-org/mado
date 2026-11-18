// The command bar on WebKit (decisions/09-ui.md §3, roadmap M6-T03): ⌘L
// in a page opens it in the overlay webview over the page, with focus.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { bin, Mado } from "./harness";

const run = bin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Win = {
  activeTabId: string | null;
  focus: string;
  contentRect: { x: number; y: number; width: number; height: number } | null;
  overlay: { kind: string; rect: { x: number; y: number; width: number; height: number } } | null;
};

run("command bar", () => {
  let mado: Mado;
  beforeAll(async () => {
    mado = await Mado.create();
    await mado.start();
  });
  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
  });

  test("⌘L opens it over the page", async () => {
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["mado://history"],
    });
    // Headless windows are never key, so the menu goes to the window that
    // was focused last: whichever opens the bar is checked.
    const windows = async () =>
      Object.values(
        (await mado.control.call<{ state: { windows: Record<string, Win> } }>("state.get")).state
          .windows,
      );
    const win = async () =>
      (await windows()).find((x) => x.overlay) ??
      ((await windows()).find((x) => (x as Win & { id: string }).id === windowId) as Win);
    let w = await win();
    for (let i = 0; i < 100 && !w.contentRect; i++) {
      await sleep(50);
      w = await win();
    }
    await sleep(500);
    await mado.control.call("test.key", { tabId: w.activeTabId, keys: "Cmd+L" });
    for (let i = 0; i < 100 && !w.overlay; i++) {
      await sleep(50);
      w = await win();
    }
    expect(w.overlay?.kind).toBe("commandBar");
    expect(w.focus).toBe("overlay");
    // Hidden test windows get no animation frames, so the UI may not have
    // reported the page yet; the bar is the command bar's size either way.
    const bar = w.overlay?.rect;
    expect(bar?.width).toBeLessThanOrEqual(640);
    expect(bar?.height).toBeLessThanOrEqual(440);
    const page = w.contentRect;
    if (page && bar) {
      expect(Math.abs(bar.x + bar.width / 2 - (page.x + page.width / 2))).toBeLessThanOrEqual(1);
      expect(bar.y).toBeGreaterThanOrEqual(page.y);
    }
  }, 60_000);
});
