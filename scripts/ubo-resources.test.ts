import { describe, expect, test } from "bun:test";
import { ordered, toResource } from "./ubo-resources";

describe("uBlock Origin resources", () => {
  test("a scriptlet becomes an fn/javascript resource with its trust", () => {
    function setConstant(chain: string, value: string) {
      return [chain, value];
    }
    const r = toResource({
      name: "set-constant.js",
      aliases: ["set"],
      fn: setConstant as never,
      dependencies: ["safe-self.fn"],
      requiresTrust: true,
    });
    expect(r.kind).toEqual({ mime: "application/javascript" });
    expect(Buffer.from(r.content, "base64").toString()).toStartWith("function setConstant(");
    expect(r.dependencies).toEqual(["safe-self.fn"]);
    expect(r.permission).toBe(1);
    expect(toResource({ name: "plain.js", fn: setConstant as never }).permission).toBeUndefined();
    expect(() => toResource({ name: "arrow.js", fn: (() => 1) as never })).toThrow();
    expect(toResource({ name: "helper.fn", fn: (() => 1) as never }).name).toBe("helper.fn");
  });

  test("dependencies come first", () => {
    const r = (name: string, dependencies?: string[]) => ({
      name,
      aliases: [],
      kind: { mime: "fn/javascript" as const },
      content: "",
      dependencies,
    });
    const out = ordered([r("a.js", ["b.fn"]), r("b.fn", ["c.fn"]), r("c.fn")]).map((x) => x.name);
    expect(out).toEqual(["c.fn", "b.fn", "a.js"]);
    expect(() => ordered([r("a.js", ["x.fn"])])).toThrow();
  });
});
