// Private windows end to end (spec 12 §1.6, roadmap M2-T09): a temporary
// profile in its own process, shared by private windows, that records no
// history and leaves nothing behind.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { readdir, readFile, stat } from "node:fs/promises";
import { join } from "node:path";
import { bin, Mado } from "./harness";

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

type S = {
  state: {
    profile: { isPrivate: boolean };
    windows: Record<string, unknown>;
    tabs: Record<string, { url: string }>;
  };
};

run("private windows", () => {
  let mado: Mado;
  let privatePid = 0;

  beforeAll(async () => {
    mado = await Mado.create();
    await mado.start();
  });

  afterAll(async () => {
    if (privatePid) {
      try {
        process.kill(privatePid, "SIGKILL");
      } catch {}
    }
    await mado?.quit();
    await mado?.remove();
  });

  test("a private window runs in a temporary profile that is deleted on close (12 §1.6)", async () => {
    const own = (await mado.control.call<{ id: string }[]>("profile.list"))[0]?.id ?? "";
    const entries = () => readdir(join(mado.dir, "Control")).catch(() => [] as string[]);
    await mado.control.call("window.new", {
      private: true,
      urls: ["https://private.mado.test/page"],
    });
    const id = await until("the private profile", async () => {
      const name = (await entries()).find((n) => n.endsWith(".json") && !n.startsWith(own));
      return name?.slice(0, -5);
    });
    const { pid } = JSON.parse(await readFile(join(mado.dir, "Control", `${id}.json`), "utf8")) as {
      pid: number;
    };
    const control = await mado.connect(id);
    const state = () => control.call<S>("state.get").then((s) => s.state);
    expect((await state()).profile.isPrivate).toBe(true);
    await until("the private tab", async () =>
      Object.values((await state()).tabs).some((t) =>
        t.url.startsWith("https://private.mado.test/"),
      )
        ? true
        : undefined,
    );
    // A second private window joins the same profile.
    await mado.control.call("window.new", { private: true, urls: [] });
    await until("a second window", async () =>
      Object.keys((await state()).windows).length === 2 ? true : undefined,
    );
    expect((await entries()).filter((n) => n.endsWith(".json")).length).toBe(2);
    const folder = join(mado.dir, "Temporary", id);
    expect(await stat(folder).catch(() => null)).not.toBeNull();
    control.close();
    process.kill(pid, "SIGTERM");
    privatePid = 0;
    await until("the folder gone", async () =>
      (await stat(folder).catch(() => null)) === null ? true : undefined,
    );
  }, 60_000);
});
