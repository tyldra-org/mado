import { expect, test } from "bun:test";
import { t } from "./index";

test("returns the English message for a known key", () => {
  expect(t("app.name")).toBe("Mado");
});

test("returns the key itself when a message is missing", () => {
  expect(t("missing.key")).toBe("missing.key");
});
