// The blocker's cosmetic filtering, scriptlets, element picker and an
// exception filter on WebKit (spec 10 §6, §8, §9 check 1; roadmap M3-T13,
// M3-T15), with custom filters and a test scriptlet resource so no list
// download is needed.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { type Subprocess, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(
  what: string,
  probe: () => Promise<T | undefined>,
  ms = 30_000,
): Promise<T> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe();
    if (v !== undefined) return v;
    await sleep(150);
  }
  throw new Error(`timed out waiting for ${what}`);
}

type Tab = { url: string; title: string };
type State = {
  tabs: Record<string, Tab>;
  windows: Record<string, { tabOrder: string[] }>;
  blocker: { ruleLists: number };
};

run("blocker cosmetics", () => {
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
    // A scriptlet resource, as the blocker's downloaded resources would hold.
    await mkdir(join(mado.dir, "Blocker"), { recursive: true });
    const scriptlet = "(function(){ window.madoScriptlet = '{{1}}'; })();";
    await writeFile(
      join(mado.dir, "Blocker", "resources.json"),
      JSON.stringify([
        {
          name: "mado-test-mark.js",
          aliases: ["mado-test-mark"],
          kind: "template",
          content: btoa(scriptlet),
        },
      ]),
    );
    await mado.start();
    await mado.control.call("settings.set", { key: "proxy.mode", value: "manual" });
    await mado.control.call("settings.set", { key: "proxy.http", value: proxy });
    await mado.control.call("blocker.setCustomFilters", {
      text: [
        "pages.mado.test##.card:has-text(Sponsored)",
        "pages.mado.test##+js(mado-test-mark, ok)",
        "||pages.mado.test/blocker/exception/",
        "@@||pages.mado.test/blocker/exception/kept.js",
      ].join("\n"),
    });
    // The shared shards and the profile's own list (the custom network
    // filters) are both in place before any page loads.
    await until(
      "the blocker",
      async () => ((await state()).blocker.ruleLists > 1 ? true : undefined),
      120_000,
    );
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
  });

  const state = async () => (await mado.control.call<{ state: State }>("state.get")).state;

  async function open(url: string): Promise<string> {
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: [url],
    });
    const tab = (await state()).windows[windowId]?.tabOrder[0];
    if (!tab) throw new Error("no tab");
    return tab;
  }

  const report = (tab: string) =>
    until("the page's report", async () => {
      const t = (await state()).tabs[tab];
      return t?.title.startsWith("sponsored:") ? t.title : undefined;
    });

  test("a procedural filter hides by text, a scriptlet runs, and the picker hides an element", async () => {
    const tab = await open("http://pages.mado.test/blocker/cosmetic.html");
    expect(await report(tab)).toBe("sponsored:hidden news:shown pick:shown scriptlet:ok");

    await mado.control.call("blocker.pickElement", { tabId: tab });
    await sleep(300);
    // #pick is the third block: under the two cards (one hidden).
    await mado.control.call("test.click", { tabId: tab, x: 50, y: 60 });
    const hidden = await until("the hidden element", async () => {
      const rows = await mado.control.call<{ selector: string }[]>("blocker.hidden", {
        host: "pages.mado.test",
      });
      return rows.length > 0 ? rows : undefined;
    });
    expect(hidden[0]?.selector).toBe("#pick");
    // It stays hidden on the next visit.
    const again = await open("http://pages.mado.test/blocker/cosmetic.html");
    expect(await report(again)).toBe("sponsored:hidden news:shown pick:hidden scriptlet:ok");
  });

  test("an exception filter lets one script through a blocked folder", async () => {
    const tab = await open("http://pages.mado.test/blocker/exception.html");
    const title = await until("the page's report", async () => {
      const t = (await state()).tabs[tab];
      return t?.title.startsWith("blocked:") ? t.title : undefined;
    });
    expect(title).toBe("blocked:blocked kept:loaded");
  });
});
