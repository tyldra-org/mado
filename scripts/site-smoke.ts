// The release smoke run (roadmap M2-T17): loads each site in
// tests/smoke/top-sites.txt in a real Mado tab over the real network, and
// fails when a tab's web process crashes or more than 5 % of the sites end
// on an error page. Run before a release on a Mac with network access:
//
//   MADO_E2E_BIN=target/debug/mado-app bun scripts/site-smoke.ts
//
// The app must be built with the test-hooks feature.
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { Mado } from "../tests/e2e/harness";

const root = join(import.meta.dir, "..");
type Tab = { url: string; title: string; life: string };
type Snapshot = {
  state: { tabs: Record<string, Tab>; windows: Record<string, { tabOrder: string[] }> };
};

/** Titles of Mado's error pages (05 §6), from en.json. */
export function errorTitles(en: Record<string, string>): Set<string> {
  return new Set(
    Object.entries(en)
      .filter(([k]) => /^errorPage\.[a-zA-Z]+\.title$/.test(k))
      .map(([, v]) => v),
  );
}

export type Outcome = "loaded" | "error" | "crashed" | "timeout";

/** What a tab's state says about its load. `waited`: the time is up, so a
 * page that shows its title counts as loaded even while slow resources
 * (ads, analytics, long polls) keep WebKit loading. */
export function outcome(tab: Tab | undefined, errors: Set<string>, waited: boolean): Outcome {
  if (!tab) return "timeout";
  if (tab.life === "crashed") return "crashed";
  if (errors.has(tab.title)) return "error";
  if (tab.life === "live") return "loaded";
  return waited && tab.title.trim() !== "" ? "loaded" : "timeout";
}

/** Passes with no crash and at most 5 % of sites failing. */
export function verdict(results: Outcome[]): boolean {
  const failed = results.filter((r) => r !== "loaded").length;
  return !results.includes("crashed") && failed <= Math.floor(results.length * 0.05);
}

async function main(): Promise<number> {
  const sites = (await readFile(join(root, "tests/smoke/top-sites.txt"), "utf8"))
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"));
  const errors = errorTitles(JSON.parse(await readFile(join(root, "ui/src/i18n/en.json"), "utf8")));
  const mado = await Mado.create();
  await mado.start();
  const results: Outcome[] = [];
  try {
    for (const site of sites) {
      const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
        private: false,
        urls: [`https://${site}/`],
      });
      const state = () => mado.control.call<Snapshot>("state.get").then((s) => s.state);
      const tabId = (await state()).windows[windowId]?.tabOrder[0] ?? "";
      let result: Outcome = "timeout";
      const end = Date.now() + 30_000;
      while (Date.now() < end) {
        result = outcome((await state()).tabs[tabId], errors, false);
        if (result !== "timeout") break;
        await new Promise((r) => setTimeout(r, 250));
      }
      const last = (await state()).tabs[tabId];
      if (result === "timeout") result = outcome(last, errors, true);
      results.push(result);
      const detail =
        result === "loaded"
          ? ""
          : ` (${last?.life ?? "gone"}: ${last?.url ?? ""} "${last?.title ?? ""}")`;
      console.log(`${result.padEnd(8)} ${site}${detail}`);
      await mado.control.call("window.close", { windowId }).catch(() => {});
    }
  } finally {
    await mado.quit();
    await mado.remove();
  }
  const loaded = results.filter((r) => r === "loaded").length;
  console.log(`site-smoke: ${loaded} of ${results.length} loaded`);
  return verdict(results) ? 0 : 1;
}

if (import.meta.main) process.exit(await main());
