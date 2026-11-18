// Subscription CLIs over ACP on WebKit (spec 13 §2.3, §7.2, roadmap
// M5-T13; 13 §13 check 4): Claude Code's and Codex's adapters are played
// by tests/e2e/fake-acp.ts, found through fake \`claude\`, \`codex\`, \`node\`
// and \`npx\` executables. The same scripted job in Mado's runtime and in
// each CLI leaves the same audit entry kinds.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { chmod, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { type Server, type Subprocess, serve, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
// MADO_ACP_REAL=claude,codex runs the job with the real CLIs and adapters
// instead of the fakes; they must be installed and logged in.
const real = (process.env.MADO_ACP_REAL ?? "").split(",").filter(Boolean);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function sse(chunks: unknown[]): Response {
  const body = chunks.map((c) => `data: ${JSON.stringify(c)}\n\n`).join("") + "data: [DONE]\n\n";
  return new Response(body, { headers: { "content-type": "text/event-stream" } });
}
const usage = { prompt_tokens: 100, completion_tokens: 10 };
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

type Task = {
  status: string;
  answer: string;
  audit: { kind: string; detail: Record<string, unknown> }[];
};

run("subscription CLIs over ACP", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;
  let model: Server<undefined>;

  beforeAll(async () => {
    const dir = await mkdtemp(join(tmpdir(), "mado-cli-"));
    const script = async (name: string, body: string) => {
      await writeFile(join(dir, name), `#!/bin/sh\n${body}\n`);
      await chmod(join(dir, name), 0o755);
    };
    await script("claude", "echo '2.1.0 (Claude Code)'");
    await script("codex", "echo 'codex-cli 0.50.0'");
    await script("node", "echo v22.0.0");
    await script("npx", `exec "${process.execPath}" "${resolve("tests/e2e/fake-acp.ts")}"`);
    if (real.length === 0) process.env.MADO_TEST_CLI_DIRS = dir;
    model = serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: async (req) => {
        if (new URL(req.url).pathname === "/v1/models")
          return Response.json({ data: [{ id: "agent-model" }] });
        const body = (await req.json()) as { messages: { role: string }[] };
        const results = body.messages.filter((m) => m.role === "tool").length;
        if (results === 0)
          return call("open", { url: "http://pages.mado.test/reader/article.html" });
        if (results === 1) return call("open", { url: "http://pages.mado.test/ai/tools.html" });
        return say("The story is by Ada Example and the shop sells a lamp.");
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
    delete process.env.MADO_TEST_CLI_DIRS;
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
    model?.stop(true);
  });

  const asUser = <T>(command: Record<string, unknown>) =>
    mado.control.call<T>("test.asUser", { command });
  const finished = async (taskId: string): Promise<Task> => {
    for (let i = 0; i < 300; i++) {
      const t = await asUser<Task>({ type: "ai.task.get", taskId });
      if (["done", "failed", "cancelled"].includes(t.status)) return t;
      await sleep(100);
    }
    throw new Error("the task did not finish");
  };
  const job = async (cli: string | null) => {
    const { taskId } = await asUser<{ taskId: string }>({
      type: "ai.task.start",
      prompt:
        "Open http://pages.mado.test/reader/article.html and http://pages.mado.test/ai/tools.html with Mado's tools, then say who wrote the story and what the shop sells.",
      cli,
    });
    return finished(taskId);
  };

  (real.length ? test : test.skip)(
    "the job with real CLIs",
    async () => {
      const mine = await job(null);
      const kinds = (t: Task) => [...new Set(t.audit.map((a) => a.kind))].sort();
      for (const cli of real) {
        const t = await job(cli);
        console.log(
          cli,
          t.status,
          t.answer,
          JSON.stringify(t.audit.map((a) => [a.kind, a.detail])),
        );
        expect(t.status).toBe("done");
        expect(kinds(t)).toEqual(kinds(mine));
      }
    },
    600_000,
  );

  (real.length ? test.skip : test)(
    "the CLIs are found, and connect",
    async () => {
      const list = await asUser<{ provider: string; ready: boolean; version: string | null }[]>({
        type: "ai.cli.list",
      });
      const by = Object.fromEntries(list.map((c) => [c.provider, c]));
      expect(by.claudeCode).toMatchObject({ ready: true, version: "2.1.0 (Claude Code)" });
      expect(by.codex?.ready).toBe(true);
      expect(by.geminiCli?.ready).toBe(false);
      await asUser({
        type: "ai.connection.add",
        kind: "subscriptionCli",
        provider: "claudeCode",
        label: null,
        baseUrl: null,
        secret: null,
      });
      await expect(
        asUser({
          type: "ai.connection.add",
          kind: "subscriptionCli",
          provider: "geminiCli",
          label: null,
          baseUrl: null,
          secret: null,
        }),
      ).resolves.toBeDefined();
      for (let i = 0; i < 100; i++) {
        const conns = Object.values(
          (
            await mado.control.call<{
              state: {
                ai: {
                  connections: Record<
                    string,
                    { provider: string; status: string; error: string | null }
                  >;
                };
              };
            }>("state.get")
          ).state.ai.connections,
        );
        const claude = conns.find((c) => c.provider === "claudeCode");
        const gemini = conns.find((c) => c.provider === "geminiCli");
        if (claude?.status === "ready" && gemini?.status === "error") {
          expect(gemini.error).toBe("notInstalled");
          return;
        }
        await sleep(100);
      }
      throw new Error("the connections were not tested");
    },
    30_000,
  );

  (real.length ? test.skip : test)(
    "the same job leaves the same audit kinds in Mado's runtime, Claude Code and Codex (13 §13 check 4)",
    async () => {
      const kinds = (t: Task) => [...new Set(t.audit.map((a) => a.kind))].sort();
      const mine = await job(null);
      expect(mine.status).toBe("done");
      for (const cli of ["claude", "codex"]) {
        const t = await job(cli);
        expect(t.answer).toContain("Ada Example");
        expect(t.status).toBe("done");
        expect(kinds(t)).toEqual(kinds(mine));
        const done = t.audit.find((a) => a.kind === "done");
        expect(done?.detail.sources).toEqual([
          "http://pages.mado.test/reader/article.html",
          "http://pages.mado.test/ai/tools.html",
        ]);
        const opened = t.audit.filter((a) => a.kind === "tool_call").map((a) => a.detail.tool);
        expect(opened).toEqual(["open", "open"]);
      }
    },
    120_000,
  );

  (real.length ? test.skip : test)(
    "a CLI's own command asks the person, and a denial reaches the agent",
    async () => {
      const { taskId } = await asUser<{ taskId: string }>({
        type: "ai.task.start",
        prompt: "PERMISSION",
        cli: "claude",
      });
      let card: { id: string; action: string; detail: string } | undefined;
      for (let i = 0; i < 200 && !card; i++) {
        const s = (
          await mado.control.call<{
            state: {
              prompts: Record<string, { id: string; type: string; action: string; detail: string }>;
            };
          }>("state.get")
        ).state;
        card = Object.values(s.prompts).find((p) => p.type === "aiApproval");
        if (!card) await sleep(50);
      }
      expect(card).toMatchObject({ action: "terminal", detail: "Run ls ~" });
      await asUser({ type: "ai.approval.respond", promptId: card?.id, answer: "deny" });
      const t = await finished(taskId);
      expect(t.answer).toBe("permission: no");
      expect(t.audit.some((a) => a.kind === "approval")).toBe(true);
    },
    60_000,
  );
});
