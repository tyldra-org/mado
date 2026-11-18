// A scripted ACP agent standing in for a subscription CLI's adapter (spec
// 13 §7.2, §13 check 4). It starts the MCP server Mado names in
// session/new and runs the job a scripted model runs in Mado's runtime:
// open two fixture pages, then answer with both as sources. A prompt with
// "PERMISSION" asks to run a command instead.
import { type Subprocess, spawn } from "bun";

type Server = { command: string; args: string[]; env: { name: string; value: string }[] };
type Msg = {
  id?: number | string;
  method?: string;
  params?: { mcpServers?: Server[]; prompt?: { text?: string }[] };
  result?: { isError?: boolean; outcome?: { optionId?: string } };
  error?: unknown;
};
const send = (v: unknown) => process.stdout.write(`${JSON.stringify(v)}\n`);

let mcp: Subprocess<"pipe", "pipe", "inherit"> | null = null;
let mcpNext = 0;
const mcpWaiting = new Map<number, (v: Msg) => void>();
const mcpCall = (method: string, params: unknown) =>
  new Promise<Msg>((resolve) => {
    const id = ++mcpNext;
    mcpWaiting.set(id, resolve);
    mcp?.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    mcp?.stdin.flush();
  });

let next = 1000;
const waiting = new Map<number, (v: Msg) => void>();
const ask = (method: string, params: unknown) =>
  new Promise<Msg>((resolve) => {
    const id = ++next;
    waiting.set(id, resolve);
    send({ jsonrpc: "2.0", id, method, params });
  });
const update = (u: unknown) =>
  send({ jsonrpc: "2.0", method: "session/update", params: { sessionId: "s1", update: u } });

async function lines(stream: ReadableStream<Uint8Array>, each: (m: Msg) => void) {
  const dec = new TextDecoder();
  let buf = "";
  for await (const chunk of stream) {
    buf += dec.decode(chunk);
    let i = buf.indexOf("\n");
    while (i >= 0) {
      const line = buf.slice(0, i).trim();
      buf = buf.slice(i + 1);
      if (line) each(JSON.parse(line));
      i = buf.indexOf("\n");
    }
  }
}

async function prompt(id: Msg["id"], text: string) {
  if (text.includes("PERMISSION")) {
    const r = await ask("session/request_permission", {
      sessionId: "s1",
      toolCall: {
        toolCallId: "t1",
        title: "Run ls ~",
        kind: "execute",
        rawInput: { command: "ls ~" },
      },
      options: [
        { optionId: "yes", name: "Allow", kind: "allow_once" },
        { optionId: "no", name: "Reject", kind: "reject_once" },
      ],
    });
    update({
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text: `permission: ${r.result?.outcome?.optionId}` },
    });
    send({ jsonrpc: "2.0", id, result: { stopReason: "end_turn" } });
    return;
  }
  const pages = [
    "http://pages.mado.test/reader/article.html",
    "http://pages.mado.test/ai/tools.html",
  ];
  for (const url of pages) {
    update({
      sessionUpdate: "tool_call",
      toolCallId: url,
      title: "mcp__mado__open",
      kind: "other",
      status: "pending",
    });
    const r = await mcpCall("tools/call", { name: "open", arguments: { url } });
    if (r.result?.isError) {
      update({
        sessionUpdate: "agent_message_chunk",
        content: { type: "text", text: `failed: ${JSON.stringify(r.result)}` },
      });
      send({ jsonrpc: "2.0", id, result: { stopReason: "end_turn" } });
      return;
    }
  }
  update({
    sessionUpdate: "agent_message_chunk",
    content: {
      type: "text",
      text: "The story is by Ada Example and the shop sells a lamp.\nSources:\n",
    },
  });
  update({
    sessionUpdate: "agent_message_chunk",
    content: { type: "text", text: pages.join("\n") },
  });
  send({ jsonrpc: "2.0", id, result: { stopReason: "end_turn" } });
}

await lines(Bun.stdin.stream(), (m) => {
  if (m.method === undefined && typeof m.id === "number") {
    waiting.get(m.id)?.(m);
    waiting.delete(m.id);
    return;
  }
  switch (m.method) {
    case "initialize":
      send({ jsonrpc: "2.0", id: m.id, result: { protocolVersion: 1, agentCapabilities: {} } });
      break;
    case "session/new": {
      const s = m.params?.mcpServers?.[0] as Server;
      const env: Record<string, string> = { ...(process.env as Record<string, string>) };
      for (const e of s.env) env[e.name] = e.value;
      mcp = spawn([s.command, ...s.args], {
        env,
        stdin: "pipe",
        stdout: "pipe",
        stderr: "inherit",
      });
      void lines(mcp.stdout, (r) => {
        if (typeof r.id === "number") {
          mcpWaiting.get(r.id)?.(r);
          mcpWaiting.delete(r.id);
        }
      });
      void mcpCall("initialize", {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "fake-acp", version: "1" },
      }).then(() => {
        mcp?.stdin.write(
          `${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`,
        );
        send({ jsonrpc: "2.0", id: m.id, result: { sessionId: "s1" } });
      });
      break;
    }
    case "session/prompt":
      void prompt(m.id, (m.params?.prompt ?? []).map((p) => p.text ?? "").join("\n"));
      break;
    default:
      if (m.id !== undefined)
        send({ jsonrpc: "2.0", id: m.id, error: { code: -32601, message: "method not found" } });
  }
});
