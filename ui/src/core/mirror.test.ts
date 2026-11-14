import { describe, expect, test } from "bun:test";
import type { Patch } from "../generated/Patch";
import type { State } from "../generated/State";
import { Mirror } from "./mirror";

function doc(rev: number, name: string): State {
  return { rev, profile: { name } } as unknown as State;
}

const rename = (rev: number, name: string): Patch => ({
  rev,
  ops: [{ op: "replace", path: "/profile/name", value: name }],
});

describe("Mirror", () => {
  test("applies patches in order and ignores old ones", async () => {
    let core = doc(1, "a");
    const mirror = new Mirror(
      async () => ({ rev: core.rev, state: core }),
      () => {},
    );
    await mirror.sync();
    mirror.receive(rename(2, "b"));
    mirror.receive(rename(2, "stale"));
    expect(mirror.state?.profile.name).toBe("b");
    expect(mirror.rev).toBe(2);
    core = doc(2, "b");
    expect(mirror.fetches).toBe(1);
  });

  test("one dropped patch causes exactly one state.get (spec 01 §6 check 6)", async () => {
    let core = doc(1, "a");
    const mirror = new Mirror(
      async () => ({ rev: core.rev, state: core }),
      () => {},
    );
    await mirror.sync();
    // rev 2 is lost; 3 and 4 arrive.
    core = doc(4, "d");
    mirror.receive(rename(3, "c"));
    mirror.receive(rename(4, "d"));
    await mirror.sync();
    expect(mirror.fetches).toBe(2);
    expect(mirror.state?.profile.name).toBe("d");
    expect(mirror.rev).toBe(4);
    mirror.receive(rename(5, "e"));
    expect(mirror.state?.profile.name).toBe("e");
    expect(mirror.fetches).toBe(2);
  });

  test("a reload recovers the whole document (spec 01 §6 check 4)", async () => {
    const core = doc(9, "z");
    const fresh = new Mirror(
      async () => ({ rev: core.rev, state: core }),
      () => {},
    );
    await fresh.sync();
    expect(fresh.state).toEqual(core);
  });
});
