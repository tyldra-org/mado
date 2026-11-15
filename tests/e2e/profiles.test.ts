// Profiles end to end (spec 12 §7 checks 1 and 2, roadmap M2-T09): a
// second profile in its own process, a link rule, and deletion.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { stat } from "node:fs/promises";
import { join } from "node:path";
import { bin, type Control, Mado } from "./harness";

const run = bin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(
  what: string,
  probe: () => Promise<T | undefined>,
  ms = 20_000,
): Promise<T> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe();
    if (v !== undefined) return v;
    await sleep(150);
  }
  throw new Error(`timed out waiting for ${what}`);
}

type Tab = { url: string };

/** Profile commands come only from Mado's own UI: send them as it would. */
const asUser = <T = unknown>(mado: Mado, type: string, params: Record<string, unknown> = {}) =>
  mado.control.call<T>("test.asUser", { command: { type, ...params } });

run("profiles", () => {
  let mado: Mado;
  let work = "";
  let workControl: Control | undefined;

  beforeAll(async () => {
    mado = await Mado.create();
    await mado.start();
  });

  afterAll(async () => {
    workControl?.close();
    await mado?.quit();
    await mado?.remove();
  });

  test("a link rule sends a link from another app to its profile (12 §7 check 1)", async () => {
    ({ id: work } = await asUser<{ id: string }>(mado, "profile.create", {
      name: "Work",
      color: "red",
      avatar: "",
    }));
    workControl = await mado.connect(work);
    await asUser(mado, "profile.linkRules.set", {
      rules: [{ pattern: "*.acme.com", profileId: work }],
    });
    const list = await mado.control.call<{ id: string; running: boolean }[]>("profile.list");
    expect(list.find((p) => p.id === work)?.running).toBe(true);
    // A link from another app: a launch with the URL, as macOS's open does.
    expect(await mado.run("https://docs.acme.com/handbook")).toBe(0);
    const control = workControl;
    await until("the link in Work", async () => {
      const s = await control.call<{ state: { tabs: Record<string, Tab> } }>("state.get");
      return Object.values(s.state.tabs).some((t) =>
        t.url.startsWith("https://docs.acme.com/handbook"),
      )
        ? true
        : undefined;
    });
    const mine = await mado.control.call<{ state: { tabs: Record<string, Tab> } }>("state.get");
    expect(Object.values(mine.state.tabs).some((t) => t.url.includes("acme.com"))).toBe(false);
  }, 60_000);

  test("deleting a profile quits it and removes its folder and website data (12 §7 check 2)", async () => {
    expect(await mado.control.call("test.hasDataStore", { profileId: work })).toBe(true);
    await asUser(mado, "profile.delete", { id: work });
    expect(await stat(join(mado.dir, "Profiles", work)).catch(() => null)).toBeNull();
    expect(await mado.control.call("test.hasDataStore", { profileId: work })).toBe(false);
    const list = await mado.control.call<{ id: string }[]>("profile.list");
    expect(list.map((p) => p.id)).not.toContain(work);
    expect(await mado.control.call("profile.linkRules")).toEqual([]);
  }, 60_000);
});
