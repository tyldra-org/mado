// `mado mcp` as an MCP client sees it (spec 14 §4, roadmap M5-T08): it
// pairs as "mado mcp", lists only the tools its scopes allow, and opens a
// page and reads its title.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { type Subprocess, spawn } from "bun";
import { bin, Mado } from "./harness";

const cli = process.env.MADO_CLI_BIN ?? "target/debug/mado";
const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

run("mado mcp", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;

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
    const proxy = /proxy=(\S+)/.exec(text)?.[1] ?? "";
    mado = await Mado.create();
    await mado.start();
    await mado.control.call("settings.set", { key: "proxy.mode", value: "manual" });
    await mado.control.call("settings.set", { key: "proxy.http", value: proxy });
    await mado.control.call("window.new", { private: false, urls: [] });
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
  });

  const asUser = <T>(command: Record<string, unknown>) =>
    mado.control.call<T>("test.asUser", { command });

  test("pairs, lists its tools and opens a page (M5-T08)", async () => {
    await mado.control.call("test.fakeAuthSuccess");
    const { token } = await asUser<{ token: string }>({ type: "app.authenticate", reason: "test" });
    await asUser({ type: "settings.set", key: "developer.controlEnabled", value: true, token });
    const server = spawn([cli, "mcp"], {
      env: { ...mado.env(), MADO_CLI_TOKENS: join(mado.dir, "cli-tokens") },
      stdin: "pipe",
      stdout: "pipe",
      stderr: "inherit",
    });
    const reader = server.stdout.getReader();
    let buffer = "";
    const messages: Record<string, unknown>[] = [];
    const pump = (async () => {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) return;
        buffer += new TextDecoder().decode(value);
        let at = buffer.indexOf("\n");
        while (at >= 0) {
          messages.push(JSON.parse(buffer.slice(0, at)));
          buffer = buffer.slice(at + 1);
          at = buffer.indexOf("\n");
        }
      }
    })();
    let next = 1;
    const ask = async (method: string, params: unknown = {}) => {
      const id = next++;
      server.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
      for (let i = 0; i < 600; i++) {
        const m = messages.find((x) => x.id === id);
        if (m) return m as { result: Record<string, unknown> };
        await sleep(50);
      }
      throw new Error(`no answer to ${method}`);
    };
    const init = await ask("initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "test", version: "0" },
    });
    expect((init.result.serverInfo as { name: string }).name).toBe("mado");
    expect(init.result.instructions).toContain("snapshot");
    // Until the person allows it, the only tool says what to do.
    const card = await (async () => {
      for (let i = 0; i < 200; i++) {
        const s = (
          await mado.control.call<{
            state: {
              prompts: Record<
                string,
                { type: string; id: string; clientName: string; offerFull: boolean }
              >;
            };
          }>("state.get")
        ).state;
        const c = Object.values(s.prompts).find((p) => p.type === "controlPairing");
        if (c) return c;
        await sleep(50);
      }
      throw new Error("no pairing card");
    })();
    expect(card.clientName).toBe("mado mcp");
    expect(card.offerFull).toBe(false);
    await asUser({
      type: "control.pairRespond",
      promptId: card.id,
      allow: true,
      scopes: ["read", "act"],
    });
    for (
      let i = 0;
      i < 100 && !messages.some((m) => m.method === "notifications/tools/list_changed");
      i++
    )
      await sleep(50);
    const list = await ask("tools/list");
    const names = (list.result.tools as { name: string }[]).map((t) => t.name);
    expect(names).toContain("open");
    expect(names).toContain("snapshot");
    const opened = await ask("tools/call", {
      name: "open",
      arguments: { url: "http://pages.mado.test/ai/tools.html" },
    });
    const text = (opened.result.content as { type: string; text: string }[])[0]?.text ?? "";
    expect(opened.result.isError).toBe(false);
    expect(JSON.parse(text).title).toBe("Tools");
    const shot = await ask("tools/call", {
      name: "screenshot",
      arguments: { tab: JSON.parse(text).tab },
    });
    expect((shot.result.content as { type: string }[])[0]?.type).toBe("image");
    server.stdin.end();
    await pump;
  }, 60_000);
});
