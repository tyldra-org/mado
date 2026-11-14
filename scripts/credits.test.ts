import { describe, expect, test } from "bun:test";
import { permitted } from "./credits";

describe("permitted", () => {
  const ok = new Set(["MIT", "Apache-2.0", "ISC"]);
  test("any alternative of an OR may satisfy it; every part of an AND must", () => {
    expect(permitted("MIT", ok)).toBe(true);
    expect(permitted("(MIT OR GPL-3.0)", ok)).toBe(true);
    expect(permitted("Apache-2.0 AND ISC", ok)).toBe(true);
    expect(permitted("MIT AND GPL-3.0", ok)).toBe(false);
    expect(permitted("UNKNOWN", ok)).toBe(false);
    expect(permitted("", ok)).toBe(false);
  });
});
