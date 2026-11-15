import { describe, expect, test } from "bun:test";
import { errorTitles, outcome, verdict } from "./site-smoke";

describe("site smoke", () => {
  const errors = errorTitles({
    "errorPage.dnsFailed.title": "This site can't be reached",
    "other.title": "x",
  });

  test("a tab's state tells loaded, error, crash and still loading apart", () => {
    expect(outcome({ url: "", title: "Home", life: "live" }, errors, true)).toBe("loaded");
    expect(
      outcome({ url: "", title: "This site can't be reached", life: "live" }, errors, true),
    ).toBe("error");
    expect(outcome({ url: "", title: "", life: "crashed" }, errors, true)).toBe("crashed");
    expect(outcome({ url: "", title: "", life: "loading" }, errors, true)).toBe("timeout");
    expect(outcome({ url: "", title: "Slack", life: "loading" }, errors, false)).toBe("timeout");
    expect(outcome({ url: "", title: "Slack", life: "loading" }, errors, true)).toBe("loaded");
    expect(outcome(undefined, errors, true)).toBe("timeout");
  });

  test("any crash fails; up to 5 % of sites may fail to load", () => {
    const ok = Array<"loaded">(95).fill("loaded");
    expect(verdict([...ok, "error", "timeout", "error", "error", "error"])).toBe(true);
    expect(verdict([...ok, "error", "timeout", "error", "error", "error", "error"])).toBe(false);
    expect(verdict([...ok, "crashed"])).toBe(false);
  });
});
