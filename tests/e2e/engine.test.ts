// The WebKit adapter end to end (spec 02 §8, roadmap M2-T02): real tab
// views load fixture pages through the fixture server's proxy.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { type Subprocess, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;

type Tab = {
  id: string;
  url: string;
  title: string;
  life: string;
  canGoBack: boolean;
  windowId: string;
};
type Win = { id: string; tabOrder: string[]; activeTabId: string | null };
type Snapshot = { state: { tabs: Record<string, Tab>; windows: Record<string, Win> } };

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(
  what: string,
  probe: () => Promise<T | undefined>,
  ms = 15_000,
): Promise<T> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe();
    if (v !== undefined) return v;
    await sleep(100);
  }
  throw new Error(`timed out waiting for ${what}`);
}

run("WebKit adapter", () => {
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;
  let proxy = "";
  let mado: Mado;

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
    proxy = /proxy=(\S+)/.exec(text)?.[1] ?? "";
    expect(proxy).not.toBe("");
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

  const state = () => mado.control.call<Snapshot>("state.get").then((s) => s.state);

  async function open(url: string): Promise<{ window: string; tab: string }> {
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: [url],
    });
    const s = await state();
    const tab = s.windows[windowId]?.tabOrder[0];
    if (!tab) throw new Error("no tab");
    return { window: windowId, tab };
  }

  const tabWhere = (id: string, check: (t: Tab) => boolean, ms?: number) =>
    until(
      `tab ${id}`,
      async () => {
        const t = (await state()).tabs[id];
        return t && check(t) ? t : undefined;
      },
      ms,
    );

  test("pages load, history and titles reach the state (02 §8 check 1, navigation events)", async () => {
    const { tab } = await open("http://pages.mado.test/nav/a.html");
    const t = await tabWhere(tab, (t) => t.title === "Page B" && t.life === "live");
    expect(t.url).toBe("http://pages.mado.test/nav/b.html");
    expect(t.canGoBack).toBe(true);
  });

  test("a page finds no Mado objects and cannot reach the bridge (02 §8 check 2)", async () => {
    const { tab } = await open("http://pages.mado.test/hostile.html");
    const t = await tabWhere(tab, (t) => t.title.startsWith("Hostile:"));
    expect(t.title).toBe("Hostile: nothing; no channel");
  });

  test("sleeping and waking keeps back and forward history (02 §8 check 3)", async () => {
    const { window, tab } = await open("http://pages.mado.test/nav/a.html");
    await tabWhere(tab, (t) => t.title === "Page B" && t.canGoBack);
    await mado.control.call("tab.new", {
      windowId: window,
      url: "http://pages.mado.test/index.html",
      position: null,
      background: false,
      openerId: null,
    });
    await mado.control.call("tab.sleep", { tabIds: [tab] });
    await tabWhere(tab, (t) => t.life === "sleeping");
    await mado.control.call("tab.activate", { tabId: tab });
    const woken = await tabWhere(tab, (t) => t.life === "live" && t.canGoBack);
    expect(woken.url).toBe("http://pages.mado.test/nav/b.html");
  });

  test("window.open without a user gesture is blocked (02 §8 check 4, the refused half)", async () => {
    const { window, tab } = await open("http://pages.mado.test/popup.html");
    const t = await tabWhere(tab, (t) => t.title.startsWith("Popup:"));
    expect(t.title).toBe("Popup: blocked");
    expect((await state()).windows[window]?.tabOrder.length).toBe(1);
  });

  test("status reports the page's icons", async () => {
    const { tab } = await open("http://pages.mado.test/scripts/favicon.html");
    const t = await tabWhere(tab, (t) => !!(t as Tab & { favicon: string | null }).favicon);
    expect((t as Tab & { favicon: string }).favicon).toBe("http://pages.mado.test/icon-32.png");
  });

  test("forms marks typed text", async () => {
    const { tab } = await open("http://pages.mado.test/scripts/forms.html");
    await tabWhere(tab, (t) => (t as Tab & { hasUnsavedInput: boolean }).hasUnsavedInput);
  });

  test("find counts matches in the page's text", async () => {
    const { window, tab } = await open("http://pages.mado.test/scripts/find.html");
    await tabWhere(tab, (t) => t.title === "Find" && t.life === "live");
    await mado.control.call("tab.find", { tabId: tab, query: "apple" });
    const bar = await until("find results", async () => {
      const w = (await state()).windows[window] as Win & {
        findBar: { query: string; matches: number | null; active: number | null } | null;
      };
      return w.findBar?.matches != null ? w.findBar : undefined;
    });
    expect(bar).toEqual({ query: "apple", matches: 3, active: 1 });
  });

  test("media mutes the page's media elements", async () => {
    const { tab } = await open("http://pages.mado.test/scripts/media.html");
    await tabWhere(tab, (t) => t.title === "Media: on" && t.life === "live");
    await mado.control.call("tab.mute", { tabIds: [tab], muted: true });
    await tabWhere(tab, (t) => t.title === "Media: muted");
  });

  test("the shims answer for the permission flow to come", async () => {
    const { tab } = await open("http://pages.mado.test/scripts/shims.html");
    const t = await tabWhere(tab, (t) => t.title.includes("geo"));
    expect(JSON.parse(t.title.slice("Shims: ".length))).toEqual({
      protocol: "registered",
      notification: "default",
      geo: 1,
    });
  });

  test("a page that stops answering gets the unresponsive prompt, which goes when it recovers (02 §5.3)", async () => {
    const { tab } = await open("http://pages.mado.test/scripts/hang.html");
    type P = { type: string; tabId?: string };
    const prompts = async () =>
      Object.values(
        (await mado.control.call<{ state: { prompts: Record<string, P> } }>("state.get")).state
          .prompts,
      );
    await until(
      "the prompt",
      async () =>
        (await prompts()).some((p) => p.type === "unresponsive" && p.tabId === tab)
          ? true
          : undefined,
      20_000,
    );
    await tabWhere(tab, (t) => t.title === "Hang: recovered", 20_000);
    await until(
      "the prompt to go",
      async () => ((await prompts()).some((p) => p.tabId === tab) ? undefined : true),
      10_000,
    );
  }, 45_000);

  test("history finds a phrase only in a page's body, never on a page with a password field (12 §7 check 3)", async () => {
    const a = await open("http://pages.mado.test/history/phrase.html");
    const b = await open("http://pages.mado.test/history/login.html");
    await tabWhere(a.tab, (t) => t.life === "live");
    await tabWhere(b.tab, (t) => t.life === "live");
    type Answer = { days: { day: string; visits: { url: string }[] }[] };
    const search = (text: string) =>
      mado.control
        .call<Answer>("history.query", { text, limit: 10 })
        .then((r) => r.days.flatMap((d) => d.visits.map((v) => v.url)));
    const found = await until("the indexed phrase", async () => {
      const urls = await search("glimmerzebra");
      return urls.length ? urls : undefined;
    });
    expect(found).toEqual(["http://pages.mado.test/history/phrase.html"]);
    await sleep(4000);
    expect(await search("shimmerotter")).toEqual([]);
    const all = await search("");
    expect(all).toContain("http://pages.mado.test/history/login.html");
  }, 30_000);

  test("a zoommtg: link never navigates (02 §8 check 5)", async () => {
    const { tab } = await open("http://pages.mado.test/applink.html");
    const t = await tabWhere(tab, (t) => t.title === "App link: still here");
    expect(t.url).toBe("http://pages.mado.test/applink.html");
  });
});
