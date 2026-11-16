// Extensions end to end (spec 09 §12, roadmap M3-T03): the real Chrome Web
// Store, so it needs the network and runs only with MADO_E2E_NETWORK=1.
// uBlock Origin Lite downloads, verifies with the store's proof, shows the
// install prompt, and Add installs it into the profile.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { stat } from "node:fs/promises";
import { join } from "node:path";
import { bin, Mado } from "./harness";

const run = bin && process.env.MADO_E2E_NETWORK === "1" ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const UBOL = "ddkjiahejlhfcafbddmgiahcphecmpfh";

async function until<T>(
  what: string,
  probe: () => Promise<T | undefined>,
  ms = 60_000,
): Promise<T> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe();
    if (v !== undefined) return v;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

type Prompt = { type: string; id: string; extensionId: string; name: string };
type State = {
  prompts: Record<string, Prompt>;
  extensions: { installed: { id: string; version: string }[] };
  profile: { id: string };
};

run("extensions from the Chrome Web Store", () => {
  let mado: Mado;

  beforeAll(async () => {
    mado = await Mado.create();
    await mado.start();
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
  });

  const state = async () => (await mado.control.call<{ state: State }>("state.get")).state;
  const asUser = (command: Record<string, unknown>) =>
    mado.control.call("test.asUser", { command });

  test("uBlock Origin Lite installs through the prompt", async () => {
    await expect(
      mado.control.call("extension.install", { id: UBOL, source: "store" }),
    ).rejects.toThrow();
    await asUser({ type: "extension.install", id: UBOL, source: "store" });
    const prompt = await until("the install prompt", async () =>
      Object.values((await state()).prompts).find((p) => p.type === "extensionInstall"),
    );
    expect(prompt.extensionId).toBe(UBOL);
    await asUser({ type: "extension.confirmInstall", promptId: prompt.id, add: true });
    const installed = await until("installed", async () =>
      (await state()).extensions.installed.find((e) => e.id === UBOL),
    );
    const profile = (await state()).profile.id;
    const manifest = join(
      mado.dir,
      "Profiles",
      profile,
      "Extensions",
      UBOL,
      installed.version,
      "manifest.json",
    );
    expect((await stat(manifest)).isFile()).toBe(true);
  });
});
