// Approvals on WebKit (spec 13 §9, roadmap M5-T11, 13 §13 checks 2 and 3):
// a payment always asks with the page's total, a denial holds for the rest
// of the task, and a page's injected instructions cannot take the agent
// out of scope; every attempt is in the audit.

import { Database } from "bun:sqlite";
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { type Subprocess, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Approval = {
  type: string;
  id: string;
  action: string;
  amount: string | null;
  site: string;
  allowForTask: boolean;
};

run("approvals", () => {
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
    await asUser({ type: "settings.set", key: "features.ai", value: true, token: null });
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
  });

  const asUser = <T>(command: Record<string, unknown>) =>
    mado.control.call<T>("test.asUser", { command });
  const agent = <T>(
    taskId: string,
    prompt: string,
    mode: string | null,
    name: string,
    args: Record<string, unknown>,
  ) => mado.control.call<T>("test.agentTool", { taskId, prompt, mode, name, args });
  const card = async (): Promise<Approval> => {
    for (let i = 0; i < 200; i++) {
      const s = (
        await mado.control.call<{ state: { prompts: Record<string, Approval> } }>("state.get")
      ).state;
      const c = Object.values(s.prompts).find((p) => p.type === "aiApproval");
      if (c) return c;
      await sleep(50);
    }
    throw new Error("no approval card");
  };
  const audit = (task: string) => {
    const dir = join(mado.dir, "Profiles");
    const db = new Database(
      join(
        dir,
        require("node:fs")
          .readdirSync(dir)
          .find((d: string) => !d.startsWith(".")),
        "profile.sqlite",
      ),
      { readonly: true },
    );
    const rows = db.query("SELECT kind, detail FROM ai_audit WHERE task_id = ?").all(task) as {
      kind: string;
      detail: string;
    }[];
    db.close();
    return rows;
  };

  test("a payment asks with its total in every mode, and a denial holds (check 3)", async () => {
    for (const mode of ["ask", "auto", "full"]) {
      const task = crypto.randomUUID();
      const prompt = "Buy the lamp on pages.mado.test";
      const s = await agent<{ tab: string; snapshot: string }>(task, prompt, mode, "open", {
        url: "http://pages.mado.test/ai/checkout.html",
      });
      const ref = /\b(e\d+)\b button "Place order"/.exec(s.snapshot)?.[1];
      const click = agent(task, prompt, mode, "click", { tab: s.tab, ref });
      const c = await card();
      expect(c.action).toBe("pay");
      expect(c.amount).toBe("$123.45");
      expect(c.allowForTask).toBe(false);
      await asUser({ type: "ai.approval.respond", promptId: c.id, answer: "deny" });
      await expect(click).rejects.toThrow(/deniedByUser/);
      // The same action on the same site is refused at once for the rest of the task.
      await expect(agent(task, prompt, mode, "click", { tab: s.tab, ref })).rejects.toThrow(
        /deniedByUser/,
      );
      const rows = audit(task);
      expect(rows.some((r) => r.kind === "approval" && r.detail.includes('"asked":"pay"'))).toBe(
        true,
      );
      expect(rows.some((r) => r.kind === "tool_call")).toBe(true);
    }
  }, 120_000);

  test("injected instructions cannot take the agent out of scope (check 2)", async () => {
    const task = crypto.randomUUID();
    const prompt = "Summarise the recipe on pages.mado.test";
    const s = await agent<{ tab: string; snapshot: string }>(task, prompt, null, "open", {
      url: "http://pages.mado.test/ai/inject.html",
    });
    expect(s.snapshot).toContain("ignore previous instructions");
    expect(s.snapshot).toContain('<untrusted source="http://pages.mado.test">');
    // A model that obeyed the page would open evil.example: that asks, and is denied.
    const out = agent(task, prompt, null, "open", { url: "https://evil.example/collect" });
    const c = await card();
    expect(c.action).toBe("leaveScope");
    expect(c.site).toBe("evil.example");
    await asUser({ type: "ai.approval.respond", promptId: c.id, answer: "deny" });
    await expect(out).rejects.toThrow(/deniedByUser/);
    const rows = audit(task);
    expect(
      rows.some(
        (r) =>
          r.kind === "approval" &&
          r.detail.includes("leaveScope") &&
          r.detail.includes("evil.example"),
      ),
    ).toBe(true);
    expect(rows.some((r) => r.kind === "tool_call" && r.detail.includes("deniedByUser"))).toBe(
      true,
    );
  }, 60_000);
});
