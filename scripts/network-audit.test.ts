import { describe, expect, test } from "bun:test";
import { allowlist, engineHosts, matches, unlisted } from "./network-audit";

describe("network audit", () => {
  test("patterns match a host exactly or a suffix and its subdomains", () => {
    expect(matches("github.com", "github.com")).toBe(true);
    expect(matches("api.github.com", "github.com")).toBe(false);
    expect(matches("mado.test", "*.mado.test")).toBe(true);
    expect(matches("pages.mado.test", "*.mado.test")).toBe(true);
    expect(matches("evilmado.test", "*.mado.test")).toBe(false);
  });

  test("unlisted hosts are reported once, sorted, case-insensitively", () => {
    expect(
      unlisted(
        ["Pages.mado.test", "tracker.example", "", "tracker.example", "a.example"],
        ["*.mado.test"],
      ),
    ).toEqual(["a.example", "tracker.example"]);
  });

  test("the bundled search engines' hosts are allowed", async () => {
    expect(engineHosts('x("https://www.google.com/complete/search?q={searchTerms}")')).toEqual([
      "www.google.com",
    ]);
    const allowed = await allowlist();
    expect(allowed).toContain("captive.apple.com");
    expect(allowed).toContain("www.google.com");
    expect(allowed).toContain("duckduckgo.com");
  });

  // 13 §13 check 1: the audit runs with AI off, so no AI provider may be
  // allowed; any request to one fails the audit.
  test("no AI provider's host is allowed", async () => {
    const allowed = await allowlist();
    for (const host of [
      "api.anthropic.com",
      "api.openai.com",
      "generativelanguage.googleapis.com",
      "api.x.ai",
      "api.mistral.ai",
      "api.deepseek.com",
      "api.groq.com",
      "openrouter.ai",
      "gateway.ai.cloudflare.com",
    ]) {
      expect(unlisted([host], allowed)).toEqual([host]);
    }
  });
});
