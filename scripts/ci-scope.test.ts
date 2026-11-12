import { describe, expect, test } from "bun:test";
import { affectedCrates, type Crate, cargoArgs, classify, EVERYTHING, unlisted } from "./ci-scope";

const crates: Crate[] = [
  { name: "mado-app", dir: "app", deps: ["mado-core", "mado-engine-webkit"], hasLib: false },
  { name: "mado-core", dir: "crates/mado-core", deps: ["mado-types"], hasLib: true },
  {
    name: "mado-engine-webkit",
    dir: "crates/mado-engine-webkit",
    deps: ["mado-engine"],
    hasLib: true,
  },
  { name: "mado-engine", dir: "crates/mado-engine", deps: ["mado-types"], hasLib: true },
  { name: "mado-types", dir: "crates/mado-types", deps: [], hasLib: true },
  { name: "mado-cli", dir: "crates/mado-cli", deps: ["mado-types"], hasLib: false },
  { name: "mado-blocker", dir: "crates/mado-blocker", deps: [], hasLib: true },
];

describe("classify", () => {
  test("docs, community files and personal tool config run nothing", () => {
    expect(
      classify([
        "README.md",
        "docs/x.md",
        ".github/ISSUE_TEMPLATE/bug.yml",
        ".github/dependabot.yml",
        ".github/VOUCHED.td",
        ".github/labeler.yml",
        "bacon.toml",
        "LICENSE",
        ".gitignore",
        "lists/README.md",
        "proto/README.md",
        ".agents/skills/mado-milestone/SKILL.md",
      ]),
    ).toEqual({
      full: false,
      rust: false,
      ts: false,
      tsGlobal: false,
      supply: false,
      shell: false,
      workflows: false,
    });
  });

  test("a crate source file runs Rust only", () => {
    const areas = classify(["crates/mado-core/src/lib.rs"]);
    expect(areas).toMatchObject({ rust: true, ts: false, supply: false });
  });

  test("a UI file runs TypeScript only, scoped to the tests it reaches", () => {
    expect(classify(["ui/src/App.svelte"])).toMatchObject({
      rust: false,
      ts: true,
      tsGlobal: false,
    });
  });

  test("dependencies and tool config run every TypeScript test", () => {
    for (const file of [
      "bun.lock",
      "ui/package.json",
      "inject/tsconfig.json",
      "ui/vite.config.ts",
      "biome.json",
    ]) {
      expect(classify([file])).toMatchObject({ ts: true, tsGlobal: true });
    }
  });

  test("mado-types also runs TypeScript, which it generates", () => {
    expect(classify(["crates/mado-types/src/lib.rs"])).toMatchObject({ rust: true, ts: true });
  });

  test("a manifest or the lockfile runs the supply chain check", () => {
    expect(classify(["crates/mado-ai/Cargo.toml"])).toMatchObject({ rust: true, supply: true });
    expect(classify(["Cargo.lock"])).toMatchObject({ rust: true, supply: true });
    expect(classify(["deny.toml"])).toMatchObject({ rust: false, supply: true });
  });

  test("test fixtures run every Rust and TypeScript test", () => {
    expect(classify(["tests/pages/form.html"])).toMatchObject({
      rust: true,
      ts: true,
      tsGlobal: true,
    });
  });

  test("the dependency rule script runs both sides", () => {
    expect(classify(["scripts/check-deps.ts"])).toMatchObject({ rust: true, ts: true });
  });

  test("shell scripts and workflows run their linters", () => {
    expect(classify(["scripts/release.sh"])).toMatchObject({ shell: true, rust: false });
    expect(classify([".github/workflows/release.yml"])).toMatchObject({ workflows: true });
  });

  test("a path no list names runs everything, never nothing", () => {
    expect(unlisted(["README.md", "lists/easylist.txt"])).toEqual(["lists/easylist.txt"]);
    expect(classify(["lists/easylist.txt"])).toEqual(EVERYTHING);
    expect(classify(["README.md", "assets/logo.svg"])).toEqual(EVERYTHING);
  });

  test("the CI wiring itself runs everything", () => {
    expect(classify([".github/workflows/ci.yml"])).toEqual(EVERYTHING);
    expect(classify(["scripts/check.sh"])).toEqual(EVERYTHING);
    expect(classify(["scripts/ci-scope.ts"])).toEqual(EVERYTHING);
  });
});

describe("affectedCrates", () => {
  test("a leaf crate affects only itself", () => {
    expect(affectedCrates(["crates/mado-blocker/src/lib.rs"], crates)).toEqual(["mado-blocker"]);
  });

  test("a crate affects everything that depends on it, transitively", () => {
    expect(affectedCrates(["crates/mado-engine/src/lib.rs"], crates)).toEqual([
      "mado-app",
      "mado-engine",
      "mado-engine-webkit",
    ]);
  });

  test("non-Rust files in the same change are ignored", () => {
    expect(affectedCrates(["ui/src/App.svelte", "crates/mado-cli/src/main.rs"], crates)).toEqual([
      "mado-cli",
    ]);
  });

  test("workspace-wide files affect every crate", () => {
    expect(affectedCrates(["Cargo.lock"], crates)).toBe("all");
    expect(affectedCrates(["rust-toolchain.toml"], crates)).toBe("all");
    expect(affectedCrates([".cargo/config.toml"], crates)).toBe("all");
    expect(affectedCrates(["tests/pages/form.html"], crates)).toBe("all");
    expect(affectedCrates([".config/nextest.toml"], crates)).toBe("all");
  });

  test("a path no list names affects every crate", () => {
    expect(affectedCrates(["crates/mado-cli/src/main.rs", "assets/logo.svg"], crates)).toBe("all");
  });

  test("no nameable crate falls back to all", () => {
    expect(affectedCrates(["scripts/check-deps.ts"], crates)).toBe("all");
  });

  test("a crate directory prefix does not match a sibling", () => {
    const nested: Crate[] = [
      { name: "a", dir: "crates/a", deps: [], hasLib: true },
      { name: "ab", dir: "crates/ab", deps: [], hasLib: true },
    ];
    expect(affectedCrates(["crates/ab/src/lib.rs"], nested)).toEqual(["ab"]);
  });
});

describe("cargoArgs", () => {
  test("all crates select the workspace", () => {
    expect(cargoArgs("all", crates)).toEqual({
      packages: "--workspace",
      docPackages: "--workspace",
    });
  });

  test("doc tests run only for crates with a library", () => {
    expect(cargoArgs(["mado-app", "mado-core"], crates)).toEqual({
      packages: "-p mado-app -p mado-core",
      docPackages: "-p mado-core",
    });
  });

  test("binary-only scopes skip doc tests", () => {
    expect(cargoArgs(["mado-cli"], crates).docPackages).toBe("");
  });

  test("a name cargo would never produce selects the workspace", () => {
    expect(cargoArgs(["mado-core", "x;id"], crates)).toEqual({
      packages: "--workspace",
      docPackages: "--workspace",
    });
  });
});
