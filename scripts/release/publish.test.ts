import { describe, expect, test } from "bun:test";
import { notes } from "./publish";

const changelog = `# Changelog

## [Unreleased]

## [0.1.0] - 2026-11-16

### Added

- Tabs.

## [0.0.1] - 2026-01-01

- Old.
`;

describe("release notes", () => {
  test("a version gets its CHANGELOG section and the opening help", () => {
    const n = notes("0.1.0", changelog);
    expect(n).toStartWith("### Added\n\n- Tabs.");
    expect(n).not.toContain("Old.");
    expect(n).toContain("Open Anyway");
  });

  test("a pre-release uses its base version; an unknown one gets one line", () => {
    expect(notes("0.1.0-beta.1", changelog)).toStartWith("### Added");
    expect(notes("0.3.0", changelog)).toStartWith("Mado 0.3.0.");
  });
});
