// AI chat on WebKit (spec 13 §5, roadmap M5-T04): a chat about the
// current page answers with that page's content. `ai-reader` reads the
// real page; a local OpenAI-compatible server stands in for the model and
// answers only when the page's text reached it.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { type Server, type Subprocess, serve, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(
  what: string,
  probe: () => Promise<T | undefined>,
  ms = 30_000,
): Promise<T> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe();
    if (v !== undefined) return v;
    await sleep(150);
  }
  throw new Error(`timed out waiting for ${what}`);
}

function sse(text: string): Response {
  const chunk = { choices: [{ index: 0, delta: { content: text } }] };
  const end = { choices: [{ index: 0, delta: {}, finish_reason: "stop" }] };
  return new Response(
    `data: ${JSON.stringify(chunk)}\n\ndata: ${JSON.stringify(end)}\n\ndata: [DONE]\n\n`,
    {
      headers: { "content-type": "text/event-stream" },
    },
  );
}

run("ai", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;
  let model: Server<undefined>;
  const bodies: string[] = [];

  beforeAll(async () => {
    model = serve({
      hostname: "127.0.0.1",
      port: 0,
      fetch: async (req) => {
        const path = new URL(req.url).pathname;
        if (path === "/v1/models") return Response.json({ data: [{ id: "local-model" }] });
        if (path === "/v1/chat/completions") {
          const body = await req.text();
          bodies.push(body);
          if (body.includes("Write a title")) return sse("Fixture story");
          if (body.includes("Improve the writing")) return sse("the cat sat on the mat.");
          return sse(
            body.includes("Paragraph 2 of the fixture article")
              ? "The story is by Ada Example."
              : "No page reached me.",
          );
        }
        return new Response("", { status: 404 });
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

  type State = {
    tabs: Record<string, { title: string }>;
    windows: Record<string, { tabOrder: string[] }>;
    ai: { roles: Record<string, { resolved: string | null }> };
  };
  const state = async () => (await mado.control.call<{ state: State }>("state.get")).state;
  const asUser = <T>(command: Record<string, unknown>) =>
    mado.control.call<T>("test.asUser", { command });

  test("a chat about the current page answers with its content (M5-T04)", async () => {
    await asUser({ type: "settings.set", key: "features.ai", value: true, token: null });
    await asUser({
      type: "ai.connection.add",
      kind: "local",
      provider: "openaiCompatible",
      label: "Test model",
      baseUrl: `http://127.0.0.1:${model.port}/v1`,
      secret: null,
    });
    await until("the chat model", async () =>
      (await state()).ai.roles.chat?.resolved ? true : undefined,
    );
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["http://pages.mado.test/reader/article.html"],
    });
    const tabId = await until("the article", async () => {
      const s = await state();
      const t = s.windows[windowId]?.tabOrder[0];
      return t && s.tabs[t]?.title.startsWith("Fixture story") ? t : undefined;
    });
    const { chatId } = await asUser<{ chatId: string }>({
      type: "ai.chat.send",
      chatId: null,
      text: "Who wrote this story?",
      context: { tabIds: [tabId], selection: null, screenshotTabId: null, files: [] },
    });
    const chat = await until("the answer", async () => {
      const c = await asUser<{ messages: { role: string; text: string }[] }>({
        type: "ai.chat.get",
        chatId,
      });
      return c.messages.length === 2 ? c : undefined;
    });
    expect(chat.messages[1]?.text).toBe("The story is by Ada Example.");
    const sent = bodies.find((b) => b.includes("Who wrote this story?")) ?? "";
    expect(sent).toContain('untrusted source=\\"http://pages.mado.test\\"');
    expect(sent).not.toContain("<nav>");
  });

  test("Improve writing replaces the selection in a textarea (M5-T05)", async () => {
    await asUser({ type: "settings.set", key: "features.textCommands", value: true, token: null });
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["http://pages.mado.test/ai/textarea.html"],
    });
    const tabId = await until("the page", async () => {
      const s = await state();
      const t = s.windows[windowId]?.tabOrder[0];
      return t && s.tabs[t]?.title === "Write" ? t : undefined;
    });
    // Select "teh cat sat on teh mat." and open the context menu on it.
    const selected = await mado.control.call<string>("test.eval", {
      tabId,
      source: `const n = document.getElementById("note");
        n.focus();
        n.setSelectionRange(10, 33);
        n.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true }));
        return n.value.slice(10, 33);`,
    });
    expect(selected).toBe("teh cat sat on teh mat.");
    // The test model, not Apple's on-device one this Mac may have.
    const model = (await state()).ai.roles.commands?.resolved ?? null;
    const { promptId } = await asUser<{ promptId: string }>({
      type: "ai.command.run",
      tabId,
      commandId: "improveWriting",
      text: "teh cat sat on teh mat.",
      editable: true,
      model,
    });
    await until("the answer", async () => {
      const p = (
        await mado.control.call<{
          state: { prompts: Record<string, { done?: boolean; text?: string }> };
        }>("state.get")
      ).state.prompts[promptId];
      return p?.done ? p : undefined;
    });
    await asUser({ type: "ai.command.respond", promptId, action: "replace", model: null });
    const value = await until("the new text", async () => {
      const v = await mado.control.call<string>("test.eval", {
        tabId,
        source: `return JSON.stringify([document.getElementById("note").value, document.getElementById("events").textContent]);`,
      });
      const [text, events] = JSON.parse(v) as [string, string];
      return text.includes("the cat sat on the mat.") ? [text, events] : undefined;
    });
    expect(value[0]).toBe("Dear Sam, the cat sat on the mat. Best, Ada");
    expect(value[1]).toBe("1");
  }, 60_000);
});
