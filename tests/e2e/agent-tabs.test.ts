// Agent tabs on WebKit (spec 13 §7.3, roadmap M5-T10): the agent works in
// its own tabs, rendered in a transparent host window, while the person's
// window keeps its active tab and focus; a person who shows the tab takes
// it over until they hand it back.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { type Subprocess, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Tab = {
  url: string;
  groupId: string | null;
  agent: { taskId: string } | null;
  windowId: string;
};
type State = {
  tabs: Record<string, Tab>;
  windows: Record<string, { activeTabId: string | null; focused: boolean }>;
  groups: Record<string, { title: string; agentTaskId: string | null }>;
  prompts: Record<string, { id: string; type: string }>;
};

run("agent tabs", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;

  beforeAll(async () => {
    fixtures = spawn([fixtureBin as string, "--pages", "tests/pages"], {
      stdout: "pipe",
      stderr: "inherit",
    });
    const reader = fixtures.stdout.getReader();
    let text = "";
    while (!text.includes("\n")) {
      const { value, done } = await reader.read();
      if (done) break;
      text += new TextDecoder().decode(value);
    }
    const proxy = /proxy=(\S+)/.exec(text)?.[1] ?? "";
    mado = await Mado.create();
    await mado.start();
    await mado.control.call("settings.set", { key: "proxy.mode", value: "manual" });
    await mado.control.call("settings.set", { key: "proxy.http", value: proxy });
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
  });

  const state = async () => (await mado.control.call<{ state: State }>("state.get")).state;
  const tool = <T>(name: string, args: Record<string, unknown>) =>
    mado.control.call<T>(`tool.${name}`, args);

  test("the agent works in its tabs while the person browses (M5-T10)", async () => {
    await mado.control.call("test.asUser", {
      command: { type: "settings.set", key: "features.ai", value: true, token: null },
    });
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["http://pages.mado.test/reader/article.html"],
    });
    await sleep(500);
    const before = (await state()).windows[windowId]?.activeTabId;
    const opened = await tool<{ tab: string; title: string }>("open", {
      url: "http://pages.mado.test/ai/tools.html",
    });
    expect(opened.title).toBe("Tools");
    const s = await state();
    const tab = s.tabs[opened.tab];
    expect(tab?.agent).not.toBeNull();
    expect(s.windows[windowId]?.activeTabId).toBe(before);
    const group = s.groups[tab?.groupId ?? ""];
    expect(group?.agentTaskId).toBe(tab?.agent?.taskId ?? "x");
    // The host window: transparent, ignores the mouse, never key.
    const host = await mado.control.call<{
      alpha: number;
      ignoresMouse: boolean;
      isKey: boolean;
      visible: boolean;
    }>("test.agentHost");
    expect(host).toEqual(
      expect.objectContaining({ alpha: 0, ignoresMouse: true, isKey: false, visible: true }),
    );
    // Its page is visible and gets animation frames (M0 experiment 12).
    const live = JSON.parse(
      await mado.control.call<string>("test.eval", {
        tabId: opened.tab,
        source: `const start = performance.now(); let frames = 0;
          await new Promise((r) => { const f = () => { frames++; if (performance.now() - start < 500) requestAnimationFrame(f); else r(); }; requestAnimationFrame(f); });
          return JSON.stringify({ state: document.visibilityState, frames });`,
      }),
    ) as { state: string; frames: number };
    expect(live.state).toBe("visible");
    expect(live.frames).toBeGreaterThan(10);
    // Acting there changes nothing in the person's window.
    const snap = await tool<{ snapshot: string }>("snapshot", { tab: opened.tab });
    const ref = /\b(e\d+)\b button "Add to cart"/.exec(snap.snapshot)?.[1];
    await tool("click", { tab: opened.tab, ref });
    expect((await state()).windows[windowId]?.activeTabId).toBe(before);
    // The person's own tabs: never acted on without approval (the person
    // denies here), never their active tab's address.
    const deny = async () => {
      for (let i = 0; i < 100; i++) {
        const c = Object.values((await state()).prompts).find((p) => p.type === "aiApproval");
        if (c) {
          await mado.control.call("test.asUser", {
            command: { type: "ai.approval.respond", promptId: c.id, answer: "deny" },
          });
          return true;
        }
        await sleep(50);
      }
      return false;
    };
    const clicked = tool("click", { tab: before, ref: "e1" });
    clicked.catch(() => {});
    expect(await deny()).toBe(true);
    await expect(clicked).rejects.toThrow(/deniedByUser/);
    const navigated = tool("navigate", { tab: before, url: "http://pages.mado.test/" });
    navigated.catch(() => {});
    void deny();
    await expect(navigated).rejects.toThrow();
    // Taking over: the person shows the agent tab, the agent pauses until handed back.
    await mado.control.call("test.asUser", {
      command: { type: "tab.activate", tabId: opened.tab },
    });
    await sleep(300);
    await expect(tool("click", { tab: opened.tab, ref })).rejects.toThrow(/userTookOver/);
    await mado.control.call("test.asUser", {
      command: { type: "ai.tab.handBack", tabId: opened.tab },
    });
    await tool("click", { tab: opened.tab, ref });
  }, 60_000);
});
