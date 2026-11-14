import { describe, expect, test } from "bun:test";
import type { PatchOp } from "../generated/PatchOp";
import { applyPatch } from "./patch";

describe("applyPatch", () => {
  test("add, replace, remove and move", () => {
    const doc: Record<string, unknown> & { a: { b: number }; keep: object } = {
      a: { b: 1 },
      list: [1, 2, 3],
      keep: { x: 1 },
    };
    const ops: PatchOp[] = [
      { op: "replace", path: "/a/b", value: 2 },
      { op: "add", path: "/a/c", value: "new" },
      { op: "remove", path: "/list/0" },
      { op: "add", path: "/list/-", value: 9 },
      { op: "move", from: "/a/c", path: "/moved" },
    ];
    const next = applyPatch(doc, ops);
    expect(next).toEqual({ a: { b: 2 }, list: [2, 3, 9], keep: { x: 1 }, moved: "new" });
    // The input is untouched; unchanged branches keep their identity.
    expect(doc.a.b).toBe(1);
    expect(next.keep).toBe(doc.keep);
  });

  test("escaped keys", () => {
    const next = applyPatch({ m: {} }, [{ op: "add", path: "/m/a~1b~0c", value: true }]);
    expect(next).toEqual({ m: { "a/b~c": true } });
  });

  test("a path into nothing throws", () => {
    expect(() => applyPatch({}, [{ op: "replace", path: "/a/b", value: 1 }])).toThrow();
  });
});
