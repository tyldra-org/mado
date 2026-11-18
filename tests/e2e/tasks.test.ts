// The Mado runtime on WebKit (spec 13 §7, roadmap M5-T12; spec 14 §6
// check 6): a multi-site research task started from the mado CLI runs to
// its answer with sources and an audit. A local OpenAI-compatible server
// plays the model with a scripted conversation.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { type Server, type Subprocess, serve, spawn } from "bun";
import { bin, Mado } from "./harness";

const cli = process.env.MADO_CLI_BIN ?? "target/debug/mado";
const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function sse(chunks: unknown[]): Response {
  const body = chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join("") + "data: [DONE]\n\n";
  return new Response(body, { headers: { "content-type": "text/event-stream" } });
}
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
                id: `c-${name}-${Math.random()}`,
                type: "function",
                function: { name, arguments: JSON.stringify(args) },
              },
            ],
          },
        },
      ],
    },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
      usage: { prompt_tokens: 100, completion_tokens: 10 },
    },
  ]);
const say = (text: string) =>
  sse([
    { choices: [{ index: 0, delta: { content: text } }] },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
      usage: { prompt_tokens: 100, completion_tokens: 10 },
    },
  ]);

run("tasks", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;
  let model: Server<undefined>;

  beforeAll(async () => {
    model = serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: async (req) => {
        const path = new URL(req.url).pathname;
        if (path === "/v1/models") return Response.json({ data: [{ id: "agent-model" }] });
        const body = (await req.json()) as { messages: { role: string; content: unknown }[] };
        const results = body.messages.filter((m) => m.role === "tool").length;
        const asked = JSON.stringify(body.messages[1]?.content ?? "");
        if (asked.includes("return its title")) {
          if (results === 0) return call("open", { url: "http://pages.mado.test/ai/tools.html" });
          return say("Tools");
        }
        if (results === 0)
          return call("open", { url: "http://pages.mado.test/reader/article.html" });
        if (results === 1) return call("open", { url: "http://pages.mado.test/ai/tools.html" });
        return say(
          "The story is by Ada Example and the shop sells a lamp.\nSources:\nhttp://pages.mado.test/reader/article.html\nhttp://pages.mado.test/ai/tools.html",
        );
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
    for (const key of ["features.ai", "features.agents"]) {
      await asUser({ type: "settings.set", key, value: true, token: null });
    }
    await mado.control.call("test.fakeAuthSuccess");
    const { token } = await asUser<{ token: string }>({ type: "app.authenticate", reason: "t" });
    await asUser({ type: "settings.set", key: "developer.controlEnabled", value: true, token });
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
  const madoCli = (...args: string[]) => {
    const p = spawn([cli, ...args], {
      env: { ...mado.env(), MADO_CLI_TOKENS: join(mado.dir, "cli-tokens") },
      stdout: "pipe",
      stderr: "pipe",
    });
    return {
      code: p.exited,
      out: new Response(p.stdout).text(),
      err: new Response(p.stderr).text(),
    };
  };
  const pair = async () => {
    for (let i = 0; i < 200; i++) {
      const s = (
        await mado.control.call<{
          state: { prompts: Record<string, { type: string; id: string }> };
        }>("state.get")
      ).state;
      const c = Object.values(s.prompts).find((p) => p.type === "controlPairing");
      if (c) {
        await asUser({
          type: "control.pairRespond",
          promptId: c.id,
          allow: true,
          scopes: ["read", "act", "tasks"],
        });
        return;
      }
      await sleep(50);
    }
  };

  test("a multi-site research task finishes with sources and an audit (M5-T12)", async () => {
    // Pair once; tasks then need no one.
    const first = madoCli("tabs");
    await pair();
    expect(await first.code).toBe(0);
    const p = madoCli("Compare the story and the shop on pages.mado.test", "--no-person");
    expect(await p.code).toBe(0);
    const out = await p.out;
    const id = /session (\S+)/.exec(out)?.[1] ?? "";
    expect(out).toContain("open http://pages.mado.test/reader/article.html");
    expect(out).toContain("The story is by Ada Example");
    const task = await asUser<{
      status: string;
      answer: string;
      audit: { kind: string; detail: Record<string, unknown> }[];
    }>({
      type: "ai.task.get",
      taskId: id,
    });
    expect(task.status).toBe("done");
    const kinds = [...new Set(task.audit.map((a) => a.kind))];
    expect(kinds).toEqual(expect.arrayContaining(["model_call", "tool_call", "done"]));
    const done = task.audit.find((a) => a.kind === "done");
    expect(done?.detail.sources).toEqual([
      "http://pages.mado.test/reader/article.html",
      "http://pages.mado.test/ai/tools.html",
    ]);
  }, 90_000);

  test('mado "open … and return its title" --no-person exits 0 with the title (14 §6 check 6)', async () => {
    const p = madoCli("open pages.mado.test/ai/tools.html and return its title", "--no-person");
    expect(await p.code).toBe(0);
    expect((await p.out).trim().split("\n").at(-1)).toBe("Tools");
  }, 60_000);
});
