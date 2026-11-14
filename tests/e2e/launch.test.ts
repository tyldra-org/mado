// The first end-to-end tests (roadmap M1-T11): launch, a second launch
// forwarding to the first, and state round trips through the control
// endpoint. Spec 01 §6 checks 1 and 2, 06 §10 check 2.
import { afterEach, describe, expect, test } from "bun:test";
import { bin, Mado, RpcError } from "./harness";

type Snapshot = {
  rev: number;
  state: {
    profile: { name: string };
    windows: Record<string, unknown>;
    settings: { values: Record<string, unknown> };
  };
};

let running: Mado[] = [];
afterEach(async () => {
  await Promise.all(running.map((m) => m.remove()));
  running = [];
});

async function mado(): Promise<Mado> {
  const m = await Mado.create();
  running.push(m);
  return m;
}

describe.skipIf(!bin)("launch", () => {
  test("the first launch creates a profile and answers state.get", async () => {
    const m = await mado();
    await m.start();
    const s = await m.control.call<Snapshot>("state.get");
    expect(s.state.profile.name).toBe("Personal");
    expect(Object.keys(s.state.windows)).toHaveLength(0);
    expect(await m.quit()).toBe(0);
  });

  test("a second launch hands over to the first and exits (01 §6 check 1)", async () => {
    const m = await mado();
    await m.start();
    expect(await m.run("https://example.com/")).toBe(0);
    const s = await m.control.call<Snapshot>("state.get");
    expect(Object.keys(s.state.windows)).toHaveLength(1);
    expect(m.process?.exitCode).toBeNull();
  });

  test("a killed owner leaves no stale lock (01 §6 check 2)", async () => {
    const m = await mado();
    await m.start();
    await m.kill();
    await m.start();
    const s = await m.control.call<Snapshot>("state.get");
    expect(s.state.profile.name).toBe("Personal");
  });
});

describe.skipIf(!bin)("state round trip", () => {
  test("a setting set through the control endpoint is in the state and survives a restart", async () => {
    const m = await mado();
    await m.start();
    await m.control.call("settings.set", { key: "appearance.theme", value: "dark", token: null });
    let s = await m.control.call<Snapshot>("state.get");
    expect(s.state.settings.values["appearance.theme"]).toBe("dark");
    expect(await m.quit()).toBe(0);
    await m.start();
    s = await m.control.call<Snapshot>("state.get");
    expect(s.state.settings.values["appearance.theme"]).toBe("dark");
  });

  test("a protected setting from a control client gets Protected and changes nothing (06 §10 check 2)", async () => {
    const m = await mado();
    await m.start();
    const before = await m.control.call<Snapshot>("state.get");
    const refused = await m.control
      .call("settings.set", { key: "blocker.enabled", value: false, token: null })
      .catch((e) => e);
    expect(refused).toBeInstanceOf(RpcError);
    expect((refused as RpcError).code).toBe(-32002);
    const after = await m.control.call<Snapshot>("state.get");
    expect(after).toEqual(before);
  });
});
