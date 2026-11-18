// Memory and skills on WebKit (spec 13 §10, §11, roadmap M5-T14; 13 §13
// check 7): memory refuses a card number, the \`fast\` model files the
// inbox and the day's filing undoes, and a saved skill is offered when an
// agent tab lands on its site. A local OpenAI-compatible server plays the
// model.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { type Server, type Subprocess, serve, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const say = (text: string) =>
  new Response(
    `data: ${JSON.stringify({ choices: [{ index: 0, delta: { content: text } }] })}\n\ndata: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: "stop" }], usage: { prompt_tokens: 10, completion_tokens: 10 } })}\n\ndata: [DONE]\n\n`,
    { headers: { "content-type": "text/event-stream" } },
  );

run("memory and skills", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;
  let model: Server<undefined>;

  beforeAll(async () => {
    model = serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: async (req) => {
        if (new URL(req.url).pathname === "/v1/models")
          return Response.json({ data: [{ id: "fast-model" }] });
        const body = JSON.stringify(await req.json());
        // The filing request: the inbox's window-seat entry goes to USER.md.
        if (body.includes("memory files") && body.includes("window seats"))
          return say(
            JSON.stringify({ files: { "USER.md": "# Travel\n- Prefers window seats\n" } }),
          );
        return say("ok");
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
    for (const key of ["features.ai", "features.memory"]) {
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
    for (let i = 0; i < 100; i++) {
      const s = (
        await mado.control.call<{
          state: { ai: { roles: Record<string, { resolved: string | null }> } };
        }>("state.get")
      ).state;
      if (s.ai.roles.fast?.resolved) break;
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
  const task = crypto.randomUUID();
  const agent = <T>(name: string, args: Record<string, unknown>) =>
    mado.control.call<T>("test.agentTool", {
      taskId: task,
      prompt: "Plan a trip on pages.mado.test",
      mode: null,
      name,
      args,
    });
  const memoryDir = () => {
    const dir = join(mado.dir, "Profiles");
    const profile = readdirSync(dir).find((d) => !d.startsWith(".")) ?? "";
    return join(dir, profile, "AI", "memory");
  };

  test("memory refuses a card number (13 §13 check 7), files the inbox and undoes the day", async () => {
    await expect(agent("remember", { text: "My card is 4111 1111 1111 1111" })).rejects.toThrow(
      /memoryCard/,
    );
    await agent("remember", { text: "Prefers window seats" });
    const inbox = readFileSync(join(memoryDir(), "inbox.md"), "utf8");
    expect(inbox).toContain("Prefers window seats");
    expect(inbox).not.toContain("4111");
    const filed = await asUser<{ day: string; files: string[] }>({ type: "ai.memory.file" });
    expect(filed.files).toEqual(["USER.md"]);
    expect(readFileSync(join(memoryDir(), "USER.md"), "utf8")).toContain("window seats");
    expect(readFileSync(join(memoryDir(), "inbox.md"), "utf8")).toBe("");
    const found = await agent<{ results: { path: string }[] }>("memory_search", {
      query: "window",
    });
    expect(found.results.map((r) => r.path)).toEqual(["USER.md"]);
    await asUser({ type: "ai.memory.undo", day: filed.day });
    expect(existsSync(join(memoryDir(), "USER.md"))).toBe(false);
    expect(readFileSync(join(memoryDir(), "inbox.md"), "utf8")).toContain("Prefers window seats");
  }, 60_000);

  test("a saved skill is offered when an agent tab lands on its site", async () => {
    const document =
      "---\nname: fixture-shop\ndescription: Buy from the fixture shop\nsites: [pages.mado.test]\n---\n1. Add the lamp to the cart.\n";
    const saving = agent("skill_save", { name: "fixture-shop", document });
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
    expect(card?.action).toBe("skill");
    expect(card?.detail).toContain("Add the lamp to the cart.");
    await asUser({ type: "ai.approval.respond", promptId: card?.id, answer: "once" });
    expect(await saving).toEqual({ saved: "fixture-shop" });
    const opened = await agent<{ tab: string; skill?: string }>("open", {
      url: "http://pages.mado.test/ai/tools.html",
    });
    expect(opened.skill).toBe("Skill available: fixture-shop");
    const again = await agent<{ skill?: string }>("navigate", {
      tab: opened.tab,
      url: "http://pages.mado.test/reader/article.html",
    });
    expect(again.skill).toBeUndefined();
    const read = await agent<{ document: string }>("skill", { name: "fixture-shop" });
    expect(read.document).toBe(document);
  }, 60_000);
});
