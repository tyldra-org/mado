// AI off on WebKit (spec 13 §13 check 1, roadmap M5-T16): with
// \`features.ai\` off, Mado starts no AI process, sends no AI request and
// puts no AI script in pages. A local OpenAI-compatible server counts the
// requests a connection made while AI was on would get.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { type Server, type Subprocess, serve, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const AI_PROCESSES = /ondevice|claude|codex|gemini|copilot|cursor|npx|node/i;

run("AI off", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;
  let model: Server<undefined>;
  const requests: string[] = [];

  beforeAll(async () => {
    model = serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: (req) => {
        requests.push(new URL(req.url).pathname);
        return Response.json({ data: [{ id: "m" }] });
      },
    });
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
    await mado.start();
    await mado.control.call("settings.set", { key: "proxy.mode", value: "manual" });
    await mado.control.call("settings.set", { key: "proxy.http", value: proxy });
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
    model?.stop(true);
  });

  const asUser = <T>(command: Record<string, unknown>) =>
    mado.control.call<T>("test.asUser", { command });
  type State = {
    features: Record<string, boolean>;
    ai: { enabled: boolean };
    windows: Record<string, { activeTabId: string | null }>;
  };
  const state = async () => (await mado.control.call<{ state: State }>("state.get")).state;
  const children = async () => {
    const ps = spawn(["ps", "-A", "-o", "ppid=,comm="], { stdout: "pipe" });
    const out = await new Response(ps.stdout).text();
    const pid = mado.process?.pid;
    return out
      .split("\n")
      .map((l) => l.trim().split(/\s+/, 2))
      .filter(([ppid]) => Number(ppid) === pid)
      .map(([, comm]) => comm ?? "");
  };
  const aiScript = async (tabId: string) =>
    mado.control.call<string>("test.eval", {
      tabId,
      source: 'return typeof globalThis[Symbol.for("mado.ai")];',
    });

  test("a new profile has AI off: no AI process, script or command (13 §13 check 1)", async () => {
    const s = await state();
    expect(s.features.ai).toBe(false);
    expect(s.ai.enabled).toBe(false);
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["http://pages.mado.test/reader/article.html"],
    });
    await sleep(1500);
    const tab = (await state()).windows[windowId]?.activeTabId ?? "";
    expect(await aiScript(tab)).toBe("undefined");
    expect((await children()).filter((c) => AI_PROCESSES.test(c))).toEqual([]);
    for (const command of [
      { type: "ai.chat.list" },
      { type: "ai.routine.list" },
      { type: "ai.memory.list" },
      { type: "ai.command.list" },
      { type: "ai.task.start", prompt: "x" },
    ]) {
      await expect(asUser(command)).rejects.toThrow(/error\.ai\.off/);
    }
  }, 60_000);

  test("turning AI off stops every request to a connection made while it was on", async () => {
    for (const key of ["features.ai", "features.agents", "features.memory", "features.routines"]) {
      await asUser({ type: "settings.set", key, value: true, token: null });
    }
    await asUser({
      type: "ai.connection.add",
      kind: "local",
      provider: "openaiCompatible",
      label: "Model",
      baseUrl: `http://127.0.0.1:${model.port}/v1`,
      secret: null,
    });
    for (let i = 0; i < 50 && requests.length === 0; i++) await sleep(100);
    expect(requests).toEqual(["/v1/models"]);
    await asUser({ type: "settings.set", key: "features.ai", value: false, token: null });
    const before = requests.length;
    for (const command of [
      { type: "ai.memory.file" },
      { type: "ai.connection.test", id: "x" },
      { type: "ai.task.start", prompt: "Summarise pages.mado.test" },
      { type: "ai.chat.send", chatId: null, text: "hi", context: null, model: null },
    ]) {
      await asUser(command).catch(() => {});
    }
    await sleep(1500);
    expect(requests.length).toBe(before);
    expect((await children()).filter((c) => AI_PROCESSES.test(c))).toEqual([]);
  }, 60_000);
});
