// A download interrupted by quit resumes after relaunch (spec 12 §7
// check 5, roadmap M2-T12), through the fixture server's slow download.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Subprocess, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type D = { id: string; state: string; filename: string; received: number; error: string | null };

run("downloads", () => {
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;
  let proxy = "";
  let mado: Mado;
  let folder = "";

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
    folder = await mkdtemp(join(tmpdir(), "mado-resume-"));
    mado = await Mado.create();
    await mado.start();
    for (const [key, value] of [
      ["proxy.mode", "manual"],
      ["proxy.http", proxy],
      ["downloads.folder", folder],
    ]) {
      await mado.control.call("settings.set", { key, value });
    }
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
  });

  const downloads = async () =>
    Object.values(
      (await mado.control.call<{ state: { downloads: Record<string, D> } }>("state.get")).state
        .downloads,
    );

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

  test("a download interrupted by quit resumes after relaunch (12 §7 check 5)", async () => {
    const bytes = 3_000_000;
    await mado.control.call("window.new", {
      private: false,
      urls: [`http://dl.mado.test/__download?name=big.bin&bytes=${bytes}&rate=300000`],
    });
    await until("bytes arriving", async () =>
      (await downloads()).find((d) => d.filename === "big.bin" && d.received > 200_000),
    );
    expect(await mado.quit()).toBe(0);

    await mado.start();
    const stopped = (await downloads()).find((d) => d.filename === "big.bin");
    expect(stopped?.state).toBe("failed");
    expect(stopped?.error).toBe("interrupted");
    await mado.control.call("download.resume", { downloadId: stopped?.id });
    await until(
      "the finished file",
      async () =>
        (await downloads()).find((d) => d.filename === "big.bin" && d.state === "complete"),
      40_000,
    );
    const file = await readFile(join(folder, "big.bin"));
    expect(file.length).toBe(bytes);
    // Byte i is i % 251: the resumed part continued where the first stopped.
    for (const i of [0, 1, 250, 251, 200_000, 1_234_567, bytes - 1]) expect(file[i]).toBe(i % 251);
  }, 90_000);
});
