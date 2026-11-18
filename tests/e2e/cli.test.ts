// The mado CLI against a real Mado (spec 14 §6 checks 2, 3, 4, 7 and 8,
// roadmap M5-T07). The person's answers on the pairing card come through
// the test hooks.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { spawn } from "bun";
import { bin, Mado } from "./harness";

const cli = process.env.MADO_CLI_BIN ?? "target/debug/mado";
const run = bin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

run("mado CLI", () => {
  let mado: Mado;

  beforeAll(async () => {
    mado = await Mado.create();
    await mado.start();
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
  });

  type State = {
    prompts: Record<string, { type: string; id: string; clientName?: string; offerFull?: boolean }>;
    settings: { values: Record<string, unknown> };
    notices: { messageKey: string }[];
  };
  const state = async () => (await mado.control.call<{ state: State }>("state.get")).state;
  const asUser = <T>(command: Record<string, unknown>) =>
    mado.control.call<T>("test.asUser", { command });
  const token = async () => {
    await mado.control.call("test.fakeAuthSuccess");
    return (await asUser<{ token: string }>({ type: "app.authenticate", reason: "test" })).token;
  };
  const mado_ = (...args: string[]) => {
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
  /** Answers the pairing card that appears for `mado`. */
  const answer = async (allow: boolean, scopes: string[]) => {
    for (let i = 0; i < 200; i++) {
      const card = Object.values((await state()).prompts).find((p) => p.type === "controlPairing");
      if (card) {
        await asUser({ type: "control.pairRespond", promptId: card.id, allow, scopes });
        return card;
      }
      await sleep(50);
    }
    throw new Error("no pairing card");
  };

  test("pairing, scopes, revoking and full control (14 §6 checks 2, 3, 4, 7, 8)", async () => {
    // Control off: not allowed.
    expect(await mado_("tabs", "--no-person").code).toBe(4);
    await asUser({
      type: "settings.set",
      key: "developer.controlEnabled",
      value: true,
      token: await token(),
    });
    // Check 2: a card; Deny is exit 4; Allow lets the call and the next run without a card.
    let p = mado_("tabs");
    await answer(false, []);
    expect(await p.code).toBe(4);
    p = mado_("tabs", "--json");
    await answer(true, ["read", "act"]);
    expect(await p.code).toBe(0);
    expect(JSON.parse(await p.out)).toBeTypeOf("object");
    p = mado_("tabs", "--no-person");
    expect(await p.code).toBe(0);
    // Check 4: no full control, a protected setting is refused.
    p = mado_("settings", "set", "blocker.enabled", "false");
    expect(await p.code).toBe(4);
    expect((await state()).settings.values["blocker.enabled"]).toBe(true);
    // Check 3: revoked, the next call fails.
    const clients = await asUser<{ id: string; name: string }[]>({ type: "control.clients" });
    const client = clients.find((c) => c.name === "mado");
    await asUser({ type: "control.revoke", clientId: client?.id });
    p = mado_("tabs");
    expect(await p.code).toBe(4);
    expect(await p.err).toContain("revoked");
    // Check 7: full control from the CLI changes a protected setting with a notice.
    await asUser({
      type: "settings.set",
      key: "developer.cliFullControl",
      value: true,
      token: await token(),
    });
    p = mado_("settings", "set", "blocker.enabled", "false");
    const card = await answer(true, ["full"]);
    expect(card.offerFull).toBe(true);
    expect(await p.code).toBe(0);
    expect((await state()).settings.values["blocker.enabled"]).toBe(false);
    expect((await state()).notices.some((n) => n.messageKey === "notice.settingChanged")).toBe(
      true,
    );
    // Check 8: passwords still need the person.
    p = mado_("password", "reveal", "x", "--no-person");
    expect(await p.code).toBe(3);
    expect(await p.out).toBe("");
    // Any command by name, and the generated groups.
    p = mado_("call", "settings.get", '{"keys":["blocker.enabled"]}');
    expect(await p.code).toBe(0);
    expect(await p.out).toContain("false");
    p = mado_("settings", "set", "blocker.enabled", "true");
    expect(await p.code).toBe(0);
    p = mado_("status", "--json");
    expect(await p.code).toBe(0);
    expect(JSON.parse(await p.out).running).toBe(true);
  }, 60_000);
});
