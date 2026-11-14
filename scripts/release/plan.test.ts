import { describe, expect, test } from "bun:test";
import { plan } from "./plan";

const date = new Date("2026-11-14T03:30:00Z");

describe("plan", () => {
  test("a run without a tag is a nightly of the next version", () => {
    expect(plan({ base: "0.1.0", date, run: 12 })).toEqual({
      version: "0.1.0-nightly.20261114.12",
      channel: "nightly",
      prerelease: true,
    });
  });

  test("tags pick stable or beta", () => {
    expect(plan({ tag: "v0.1.0", base: "0.1.0", date, run: 1 }).channel).toBe("stable");
    expect(plan({ tag: "v0.1.0-beta.2", base: "0.1.0", date, run: 1 })).toEqual({
      version: "0.1.0-beta.2",
      channel: "beta",
      prerelease: true,
    });
  });

  test("refuses tags that do not match the app's version", () => {
    expect(() => plan({ tag: "v0.2.0", base: "0.1.0", date, run: 1 })).toThrow();
    expect(() => plan({ tag: "v0.1.0-nightly.1", base: "0.1.0", date, run: 1 })).toThrow();
    expect(() => plan({ tag: "v0.1.0-beta.", base: "0.1.0", date, run: 1 })).toThrow();
  });
});
