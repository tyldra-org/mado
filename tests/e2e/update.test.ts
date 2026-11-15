// An update relaunch end to end (spec 15 §5, roadmap M2-T16): every
// running profile quits, the app restarts, and each profile comes back with
// its last session, whatever startup.mode says.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { bin, type Control, Mado } from "./harness";

const run = bin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(
  what: string,
  probe: () => Promise<T | undefined>,
  ms = 30_000,
): Promise<T> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe().catch(() => undefined);
    if (v !== undefined) return v;
    await sleep(200);
  }
  throw new Error(`timed out waiting for ${what}`);
}

type S = { state: { windows: Record<string, unknown>; tabs: Record<string, { url: string }> } };

const pidOf = async (mado: Mado, id: string) =>
  (JSON.parse(await readFile(join(mado.dir, "Control", `${id}.json`), "utf8")) as { pid: number })
    .pid;

run("update relaunch", () => {
  let mado: Mado;
  const pids: number[] = [];

  beforeAll(async () => {
    mado = await Mado.create();
    await mado.start();
  });

  afterAll(async () => {
    for (const pid of pids) {
      try {
        process.kill(pid, "SIGKILL");
      } catch {}
    }
    await mado?.kill();
    await mado?.remove();
  });

  test("a relaunch brings every profile back with its session (15 §5)", async () => {
    const main =
      (await mado.control.call<{ id: string; current: boolean }[]>("profile.list")).find(
        (p) => p.current,
      )?.id ?? "";
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["https://restore.mado.test/main"],
    });
    expect(windowId).toBeTruthy();
    const { id: work } = await mado.control.call<{ id: string }>("test.asUser", {
      command: { type: "profile.create", name: "Work", color: "red", avatar: "" },
    });
    let workControl: Control = await mado.connect(work);
    await workControl.call("window.new", {
      private: false,
      urls: ["https://restore.mado.test/work"],
    });
    const before = { main: await pidOf(mado, main), work: await pidOf(mado, work) };
    await mado.control.call("test.relaunchForUpdate");
    // New processes for both profiles.
    await until("main again", async () =>
      (await pidOf(mado, main)) !== before.main ? true : undefined,
    );
    await until("work again", async () =>
      (await pidOf(mado, work)) !== before.work ? true : undefined,
    );
    pids.push(await pidOf(mado, main), await pidOf(mado, work));
    const mainControl = await mado.connect(main);
    workControl = await mado.connect(work);
    const has = (c: Control, url: string) =>
      until(url, async () => {
        const s = await c.call<S>("state.get");
        return Object.values(s.state.tabs).some((t) => t.url === url) ? true : undefined;
      });
    await has(mainControl, "https://restore.mado.test/main");
    await has(workControl, "https://restore.mado.test/work");
    mainControl.close();
    workControl.close();
  }, 90_000);
});
