// The extension matrix (spec 16 §1, 09 §12 check 2): for each
// tests/extensions/<id>.yaml, a fresh Mado installs the extension from the
// real Chrome Web Store through its prompt, runs the file's page checks on
// fixture pages, opens its popup, and checks the store for an update. Every
// extension the compatibility list marks `tested` must have a file. Needs
// the network; run by hand on a Mac:
//
//   bun run test:extensions [<id> ...]
//
// The app must be built with the test-hooks feature.
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { spawn } from "bun";
import { Mado } from "../tests/e2e/harness";

const root = join(import.meta.dir, "..");
const dir = join(root, "tests/extensions");
const ID = /^[a-p]{32}$/;

/** One page check: open `open` on pages.mado.test, then the page's title
 * must match the `title` pattern, or differ from `titleNot`. */
export type Check = { open: string; title?: string; titleNot?: string };
export type Entry = { id: string; name: string; popup: boolean; checks: Check[] };

/** Reads one matrix file; throws on anything it does not know. */
export function parseEntry(id: string, text: string): Entry {
  if (!ID.test(id)) throw new Error(`${id}: not an extension ID`);
  const raw = Bun.YAML.parse(text) as Record<string, unknown> | null;
  if (!raw || typeof raw.name !== "string") throw new Error(`${id}: needs a name`);
  const known = new Set(["name", "popup", "checks"]);
  const extra = Object.keys(raw).filter((k) => !known.has(k));
  if (extra.length > 0) throw new Error(`${id}: unknown keys ${extra.join(", ")}`);
  const checks = (raw.checks ?? []) as Check[];
  if (!Array.isArray(checks)) throw new Error(`${id}: checks must be a list`);
  for (const c of checks) {
    if (typeof c.open !== "string" || !c.open.startsWith("/"))
      throw new Error(`${id}: a check needs an open path`);
    if ((c.title === undefined) === (c.titleNot === undefined))
      throw new Error(`${id}: a check needs title or titleNot`);
  }
  return { id, name: raw.name, popup: raw.popup === true, checks };
}

/** `tested` extensions of the compatibility list without a matrix file. */
export function missing(compat: { extensions: Record<string, { status: string }> }, ids: string[]) {
  const have = new Set(ids);
  return Object.entries(compat.extensions)
    .filter(([id, e]) => e.status === "tested" && !have.has(id))
    .map(([id]) => id);
}

/** Whether a settled page title passes `check`. */
export function titlePasses(check: Check, title: string): boolean {
  if (check.title !== undefined) return new RegExp(check.title).test(title);
  return title !== check.titleNot;
}

/** The newer version the store's update service offers for `id`, or null
 * when the installed one is current. */
export function offered(xml: string, id: string): string | null {
  const app = new RegExp(`<app[^>]*appid="${id}"[^>]*>([\\s\\S]*?)</app>`).exec(xml)?.[1];
  const check = app ? /<updatecheck([^>]*?)\/?>/.exec(app)?.[1] : undefined;
  if (check === undefined) throw new Error(`${id}: no updatecheck in the answer`);
  const attr = (n: string) => new RegExp(`${n}="([^"]*)"`).exec(check)?.[1];
  const status = attr("status");
  if (status === "noupdate") return null;
  if (status !== "ok") throw new Error(`${id}: update status ${status}`);
  return attr("version") ?? null;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until<T>(what: string, probe: () => Promise<T | undefined>, ms = 60_000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe();
    if (v !== undefined) return v;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

type Prompt = { type: string; id: string; extensionId?: string; default?: string };
type State = {
  prompts: Record<string, Prompt>;
  extensions: {
    installed: { id: string; version: string }[];
    toolbar: { id: string; hasPopup: boolean }[];
  };
  tabs: Record<string, { title: string }>;
  windows: Record<string, { tabOrder: string[] }>;
};

/** Installs and checks one extension in a fresh Mado; returns what failed. */
async function run(entry: Entry, proxy: string, chromium: string): Promise<string[]> {
  const failed: string[] = [];
  const mado = await Mado.create();
  await mado.start();
  const state = async () => (await mado.control.call<{ state: State }>("state.get")).state;
  const asUser = (command: Record<string, unknown>) =>
    mado.control.call("test.asUser", { command });
  const step = async (name: string, body: () => Promise<string | undefined>) => {
    try {
      const note = await body();
      console.log(`  ok    ${name}${note ? ` (${note})` : ""}`);
    } catch (e) {
      failed.push(name);
      console.log(`  FAIL  ${name}: ${e instanceof Error ? e.message : e}`);
    }
  };
  try {
    await mado.control.call("settings.set", { key: "proxy.mode", value: "manual" });
    await mado.control.call("settings.set", { key: "proxy.http", value: proxy });
    let version = "";
    await step("install", async () => {
      await asUser({ type: "extension.install", id: entry.id, source: "store" });
      const prompt = await until("the install prompt", async () =>
        Object.values((await state()).prompts).find(
          (p) => p.type === "extensionInstall" && p.extensionId === entry.id,
        ),
      );
      await asUser({ type: "extension.confirmInstall", promptId: prompt.id, add: true });
      version = (
        await until("installed", async () =>
          (await state()).extensions.installed.find((e) => e.id === entry.id),
        )
      ).version;
      // Overlap and takeover prompts get their defaults, as most people
      // answer them.
      await sleep(3000);
      for (const p of Object.values((await state()).prompts)) {
        if (p.extensionId !== entry.id) continue;
        if (p.type === "overlap")
          await asUser({ type: "extension.overlapRespond", promptId: p.id, choice: p.default });
        if (p.type === "takeover")
          await asUser({ type: "extension.takeoverRespond", promptId: p.id, keep: true });
      }
      return version;
    });
    if (failed.length > 0) return failed;
    for (const check of entry.checks) {
      await step(`page ${check.open}`, async () => {
        // An extension sets itself up after install (its rules, its
        // scripts), so a failing page is tried again for a while.
        const end = Date.now() + 60_000;
        for (;;) {
          const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
            private: false,
            urls: [`http://pages.mado.test${check.open}`],
          });
          const title = await until("the page's title", async () => {
            const s = await state();
            const tab = s.windows[windowId]?.tabOrder[0];
            const t = tab ? s.tabs[tab]?.title : undefined;
            return t && t !== "pending" ? t : undefined;
          });
          await mado.control.call("window.close", { windowId }).catch(() => {});
          if (titlePasses(check, title)) return title;
          if (Date.now() > end) throw new Error(`title "${title}"`);
          await sleep(3000);
        }
      });
    }
    if (entry.popup) {
      await step("popup", async () => {
        const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
          private: false,
          urls: ["http://pages.mado.test/extensions/plain.html"],
        });
        await until("its toolbar button", async () =>
          (await state()).extensions.toolbar.find((b) => b.id === entry.id && b.hasPopup),
        );
        await mado.control.call("extension.openPopup", {
          id: entry.id,
          windowId,
          anchor: { x: 600, y: 10, width: 24, height: 24 },
        });
        return until("the popup", async () => {
          const u = await mado.control.call<string | null>("test.extensionPopup");
          return u?.startsWith("webkit-extension://") ? u : undefined;
        });
      });
    }
    await step("update", async () => {
      const url = `https://clients2.google.com/service/update2/crx?prodversion=${chromium}&acceptformat=crx3&x=id%3D${entry.id}%26v%3D${version}%26uc`;
      const answer = await fetch(url);
      if (!answer.ok) throw new Error(`the update service answered ${answer.status}`);
      const newer = offered(await answer.text(), entry.id);
      if (newer === null) return `${version} is current`;
      await mado.control.call("test.extensionUpdateCheck");
      await until(`the update to ${newer}`, async () =>
        (await state()).extensions.installed.find((e) => e.id === entry.id && e.version === newer),
      );
      return `updated to ${newer}`;
    });
  } finally {
    await mado.quit();
    await mado.remove();
  }
  return failed;
}

async function main(only: string[]): Promise<number> {
  const files = (await readdir(dir)).filter((f) => f.endsWith(".yaml"));
  const entries = await Promise.all(
    files.map(async (f) => parseEntry(f.slice(0, -5), await readFile(join(dir, f), "utf8"))),
  );
  const compat = JSON.parse(await readFile(join(root, "lists/compat.json"), "utf8"));
  const lacking = missing(
    compat,
    entries.map((e) => e.id),
  );
  if (lacking.length > 0) {
    console.error(`tested extensions without a matrix file: ${lacking.join(", ")}`);
    return 1;
  }
  const chromium =
    /CHROMIUM_VERSION: &str = "([^"]+)"/.exec(
      await readFile(join(root, "crates/mado-extensions/src/store.rs"), "utf8"),
    )?.[1] ?? "";
  const fixtureBin = process.env.MADO_FIXTURE_BIN;
  if (!fixtureBin) throw new Error("MADO_FIXTURE_BIN is not set");
  const fixtures = spawn([fixtureBin, "--pages", join(root, "tests/pages")], {
    stdout: "pipe",
    stderr: "inherit",
  });
  let failures = 0;
  try {
    const reader = fixtures.stdout.getReader();
    let text = "";
    while (!text.includes("\n")) {
      const { value, done } = await reader.read();
      if (done) break;
      text += new TextDecoder().decode(value);
    }
    const proxy = /proxy=(\S+)/.exec(text)?.[1];
    if (!proxy) throw new Error("the fixture server gave no proxy address");
    for (const entry of entries) {
      if (only.length > 0 && !only.includes(entry.id)) continue;
      console.log(`${entry.name} (${entry.id})`);
      const failed = await run(entry, proxy, chromium);
      if (failed.length > 0) failures += 1;
    }
  } finally {
    fixtures.kill();
  }
  console.log(
    failures === 0 ? "extension-matrix: all passed" : `extension-matrix: ${failures} failed`,
  );
  return failures === 0 ? 0 : 1;
}

if (import.meta.main) process.exit(await main(process.argv.slice(2)));
