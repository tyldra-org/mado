// The WebKit adapter end to end (spec 02 §8, roadmap M2-T02): real tab
// views load fixture pages through the fixture server's proxy.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
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

  // The fixture server is http here (the app does not trust its test CA), so
  // the .dmg is held as an insecure download rather than a dangerous type;
  // both wait for Keep (08 §4). The dangerous-type rule is a core test.
  test("an http .dmg waits for Keep, then has the quarantine mark (08 §12 check 3)", async () => {
    const folder = await mkdtemp(join(tmpdir(), "mado-dl-"));
    await mado.control.call("settings.set", { key: "downloads.folder", value: folder });
    await open("http://dl.mado.test/__download?name=Setup.dmg&bytes=2048");
    type D = {
      id: string;
      state: string;
      filename: string;
      danger: string | null;
      received: number;
    };
    const downloads = async () =>
      Object.values(
        (await mado.control.call<{ state: { downloads: Record<string, D> } }>("state.get")).state
          .downloads,
      );
    const held = await until("the held download", async () =>
      (await downloads()).find((d) => d.filename === "Setup.dmg" && d.state === "blocked"),
    );
    expect(held).toMatchObject({ danger: "insecure", received: 2048 });
    await mado.control.call("download.keep", { downloadId: held.id });
    await until("the kept download", async () =>
      (await downloads()).find((d) => d.id === held.id && d.state === "complete"),
    );
    const file = join(folder, "Setup.dmg");
    expect((await readFile(file)).length).toBe(2048);
    const xattr = spawn(["/usr/bin/xattr", "-p", "com.apple.quarantine", file], { stdout: "pipe" });
    expect(await new Response(xattr.stdout).text()).toStartWith("0081;");
  }, 30_000);

  test("a dropped connection and a redirect loop show their error pages (05 §12 check 5)", async () => {
    for (const [path, title] of [
      ["/__reset", "The connection was reset"],
      ["/__loop", "Too many redirects"],
    ]) {
      const url = `http://pages.mado.test${path}`;
      const { tab } = await open(url);
      // The title comes first; the URL settles on the failed one.
      await tabWhere(tab, (t) => t.title === title && t.url === url);
    }
  });

  test("link modifiers choose where a clicked link opens (05 §12 check 6)", async () => {
    const { window, tab } = await open("http://pages.mado.test/links.html");
    await tabWhere(tab, (t) => t.title === "Links" && t.life === "live");
    const target = "http://pages.mado.test/nav/b.html?from=link";
    const click = (mods: Record<string, boolean>) =>
      mado.control.call("test.click", { tabId: tab, x: 40, y: 40, ...mods });
    const urls = async (w: string) => {
      const s = await state();
      return (s.windows[w]?.tabOrder ?? []).map((id) => s.tabs[id]?.url);
    };
    await click({ command: true });
    await until("a background tab", async () =>
      (await urls(window)).includes(target) ? true : undefined,
    );
    expect((await state()).windows[window]?.activeTabId).toBe(tab);
    const windows = Object.keys((await state()).windows).length;
    await click({ shift: true });
    await until("a new window", async () =>
      Object.keys((await state()).windows).length === windows + 1 ? true : undefined,
    );
    expect((await state()).tabs[tab]?.url).toBe("http://pages.mado.test/links.html");
    await click({});
    await tabWhere(tab, (t) => t.url === target);
  }, 30_000);

  test("a click that opens a pop-up gets a tab (02 §8 check 4, the allowed half)", async () => {
    const { window, tab } = await open("http://pages.mado.test/links.html");
    await tabWhere(tab, (t) => t.title === "Links" && t.life === "live");
    await mado.control.call("test.click", { tabId: tab, x: 40, y: 190 });
    const s = await until("the pop-up tab", async () => {
      const s = await state();
      return (s.windows[window]?.tabOrder.length ?? 0) === 2 ? s : undefined;
    });
    const popup = s.windows[window]?.tabOrder.find((id) => id !== tab) ?? "";
    await tabWhere(popup, (t) => t.url === "http://pages.mado.test/nav/b.html?from=popup");
  }, 30_000);

  test("a clicked zoommtg: link asks with Zoom's name and icon (07 §10 check 4)", async () => {
    const { tab } = await open("http://pages.mado.test/links.html");
    await tabWhere(tab, (t) => t.title === "Links" && t.life === "live");
    await mado.control.call("test.click", { tabId: tab, x: 40, y: 340 });
    type P = { type: string; id: string; tabId: string; appName?: string; appIcon?: string | null };
    type N = { messageKey: string };
    const found = await until("the prompt or the no-app notice", async () => {
      const s = (await mado.control.call("state.get")) as {
        state: { prompts: Record<string, P>; notices: N[] };
      };
      const p = Object.values(s.state.prompts).find((p) => p.type === "appLink" && p.tabId === tab);
      if (p) return p;
      return s.state.notices.some((n) => n.messageKey === "notice.noAppForLink") ? null : undefined;
    });
    if (found === null) {
      console.warn("Zoom is not installed on this Mac: 07 §10 check 4 not shown");
      return;
    }
    expect(found.appName ?? "").toMatch(/zoom/i);
    expect(found.appIcon ?? "").toStartWith("data:image/png;base64,");
    await mado.control.call("prompt.dismiss", { id: found.id });
  }, 30_000);

  test("a cross-origin frame's alert shows nothing; the page's own asks (07 §10 check 5)", async () => {
    const { tab } = await open("http://pages.mado.test/dialogs/top.html");
    type P = { type: string; id: string; tabId: string; message?: string };
    const dialogs = async () =>
      Object.values(
        ((await mado.control.call("state.get")) as { state: { prompts: Record<string, P> } }).state
          .prompts,
      ).filter((p) => p.type === "dialog" && p.tabId === tab);
    const shown = await until("the page's alert", async () => {
      const d = await dialogs();
      return d.length ? d : undefined;
    });
    // Only the page's own: the frame's alert was answered unseen, so the
    // page heard from the frame.
    expect(shown.map((d) => d.message)).toEqual(["From the page"]);
    await mado.control.call("prompt.answerDialog", { id: shown[0]?.id, accept: true });
    await tabWhere(tab, (t) => t.title === "Dialogs: top answered");
  }, 30_000);

  test("a checkout asking for Apple Pay hears no and is offered Safari (07 §9)", async () => {
    const { tab } = await open("http://pages.mado.test/checkout.html");
    await tabWhere(tab, (t) => t.title === "Checkout: false");
    type P = { type: string; tabId: string; reason?: string };
    await until("the offer", async () => {
      const s = (await mado.control.call("state.get")) as { state: { prompts: Record<string, P> } };
      return Object.values(s.state.prompts).some(
        (p) => p.type === "openInSafari" && p.tabId === tab && p.reason === "applePay",
      )
        ? true
        : undefined;
    });
  });

  test("a bad certificate shows the interstitial; Proceed loads it; an HSTS host has no Proceed (08 §12 check 2)", async () => {
    // The app does not trust the fixture server's test CA: every https
    // fixture is a certificate error.
    const { tab } = await open("https://expired.mado.test/index.html");
    await tabWhere(tab, (t) => t.title === "This connection isn't private");
    await mado.control.call("test.errorAction", { tabId: tab, action: "proceed" });
    const t = await tabWhere(
      tab,
      (t) =>
        t.title !== "This connection isn't private" &&
        t.title !== "" &&
        (t as Tab & { security: { level: string } }).security.level === "dangerous",
    );
    expect(t.url).toBe("https://expired.mado.test/index.html");
    expect((t as Tab & { security: { level: string } }).security.level).toBe("dangerous");
    const hsts = await open("https://hsts.mado.test/index.html");
    await tabWhere(hsts.tab, (t) => t.title === "hsts.mado.test can't be opened safely");
  }, 30_000);

  test("a zoommtg: link never navigates (02 §8 check 5)", async () => {
    const { tab } = await open("http://pages.mado.test/applink.html");
    const t = await tabWhere(tab, (t) => t.title === "App link: still here");
    expect(t.url).toBe("http://pages.mado.test/applink.html");
  });
});
