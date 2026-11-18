// Routines on WebKit (spec 13 §12, roadmap M5-T15): a readOnly routine's
// agent cannot place an order and gets no card, and a run that finds
// nothing new leaves nothing in the task list. A local OpenAI-compatible
// server plays the model.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { type Server, type Subprocess, serve, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function sse(chunks: unknown[]): Response {
  const body = `${chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join("")}data: [DONE]\n\n`;
  return new Response(body, { headers: { "content-type": "text/event-stream" } });
}
const usage = { prompt_tokens: 10, completion_tokens: 10 };
const call = (name: string, args: unknown) =>
  sse([
    {
      choices: [
        {
          index: 0,
          delta: {
            tool_calls: [
              {
                index: 0,
                id: `c-${Math.random()}`,
                type: "function",
                function: { name, arguments: JSON.stringify(args) },
              },
            ],
          },
        },
      ],
    },
    { choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }], usage },
  ]);
const say = (text: string) =>
  sse([
    { choices: [{ index: 0, delta: { content: text } }] },
    { choices: [{ index: 0, delta: {}, finish_reason: "stop" }], usage },
  ]);

type Msg = { role: string; content: unknown };
type Task = {
  status: string;
  answer: string;
  audit: { kind: string; detail: Record<string, unknown> }[];
};

run("routines", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;
  let model: Server<undefined>;

  beforeAll(async () => {
    model = serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: async (req) => {
        if (new URL(req.url).pathname === "/v1/models")
          return Response.json({ data: [{ id: "agent-model" }] });
        const body = (await req.json()) as { messages: Msg[] };
        const asked = JSON.stringify(body.messages);
        const results = body.messages.filter((m) => m.role === "tool");
        if (asked.includes("Check for news")) return say("NOTHING_NEW");
        if (results.length === 0)
          return call("open", { url: "http://pages.mado.test/ai/checkout.html" });
        if (results.length === 1) {
          const ref = /\b(e\d+)\b button \\"Place order\\"/.exec(
            JSON.stringify(results[0]?.content),
          )?.[1];
          const tab = /"tab":"([^"]+)"/.exec(String(results[0]?.content))?.[1];
          return call("click", { tab, ref });
        }
        return say(`I could not order: ${String(results[1]?.content)}`);
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
    await mado.control.call("window.new", { private: false, urls: [] });
    for (const key of ["features.ai", "features.agents", "features.routines"]) {
      await asUser({ type: "settings.set", key, value: true, token: null });
    }
    await asUser({
      type: "ai.connection.add",
      kind: "local",
      provider: "openaiCompatible",
      label: "Agent model",
      baseUrl: `http://127.0.0.1:${model.port}/v1`,
      secret: null,
    });
    for (let i = 0; i < 100; i++) {
      const s = (
        await mado.control.call<{
          state: { ai: { roles: Record<string, { resolved: string | null }> } };
        }>("state.get")
      ).state;
      if (s.ai.roles.tasks?.resolved) break;
      await sleep(100);
    }
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
    model?.stop(true);
  });

  const asUser = <T>(command: Record<string, unknown>) =>
    mado.control.call<T>("test.asUser", { command });
  const state = async () =>
    (
      await mado.control.call<{
        state: {
          prompts: Record<string, { type: string }>;
          ai: { tasks: Record<string, unknown> };
        };
      }>("state.get")
    ).state;
  const finished = async (taskId: string): Promise<Task> => {
    for (let i = 0; i < 300; i++) {
      const t = await asUser<Task>({ type: "ai.task.get", taskId });
      if (["done", "failed", "cancelled"].includes(t.status)) return t;
      await sleep(100);
    }
    throw new Error("the task did not finish");
  };

  test("a readOnly routine never acts consequentially and shows no card", async () => {
    const { routineId } = await asUser<{ routineId: string }>({
      type: "ai.routine.create",
      prompt: "Buy the lamp on pages.mado.test",
      schedule: "daily 09:00",
      mode: "readOnly",
    });
    const { taskId } = await asUser<{ taskId: string }>({ type: "ai.routine.runNow", routineId });
    let cards = 0;
    for (let i = 0; i < 40; i++) {
      cards += Object.values((await state()).prompts).filter((p) => p.type === "aiApproval").length;
      await sleep(50);
    }
    const t = await finished(taskId);
    expect(cards).toBe(0);
    expect(t.answer).toContain("readOnlyRoutine");
    expect(
      t.audit.some((a) => a.kind === "tool_call" && a.detail.error === "error.ai.readOnlyRoutine"),
    ).toBe(true);
    const [r] = await asUser<{ lastRunAt: number | null; nextRunAt: number | null }[]>({
      type: "ai.routine.list",
    });
    expect(r?.lastRunAt).not.toBeNull();
    expect(r?.nextRunAt).toBeGreaterThan(Date.now());
  }, 60_000);

  test("a run that finds nothing new leaves no task behind", async () => {
    const { routineId } = await asUser<{ routineId: string }>({
      type: "ai.routine.create",
      prompt: "Check for news on pages.mado.test",
      schedule: "hourly",
      mode: null,
    });
    const { taskId } = await asUser<{ taskId: string }>({ type: "ai.routine.runNow", routineId });
    const t = await finished(taskId);
    expect(t.answer).toBe("NOTHING_NEW");
    for (let i = 0; i < 50 && (await state()).ai.tasks[taskId]; i++) await sleep(50);
    expect((await state()).ai.tasks[taskId]).toBeUndefined();
  }, 60_000);
});
