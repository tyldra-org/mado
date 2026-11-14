import { describe, expect, test } from "bun:test";
import { findViolations } from "./check-deps";

describe("findViolations", () => {
  test("accepts the intended direction", () => {
    expect(
      findViolations({
        "mado-app": ["mado-core", "mado-engine-webkit"],
        "mado-core": ["mado-engine", "mado-types"],
        "mado-engine": ["mado-types"],
        "mado-cli": ["mado-types"],
        "mado-types": [],
      }),
    ).toEqual([]);
  });

  test("rejects the core depending on an engine adapter", () => {
    expect(findViolations({ "mado-core": ["mado-engine-webkit"] })).toHaveLength(1);
  });

  test("lets mado-types use only its own derive", () => {
    expect(findViolations({ "mado-types": ["mado-types-derive"] })).toEqual([]);
    expect(findViolations({ "mado-types": ["mado-storage"] })).toHaveLength(1);
    expect(findViolations({ "mado-types-derive": ["mado-types"] })).toHaveLength(1);
  });

  test("rejects the CLI depending on anything but mado-types", () => {
    expect(findViolations({ "mado-cli": ["mado-core"] })).toHaveLength(1);
  });

  test("rejects anything depending on the app", () => {
    expect(findViolations({ "mado-agent": ["mado-app"] })).toHaveLength(1);
  });
});
