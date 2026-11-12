// Decides what CI has to check for a change, from the files it touches.
// Usage:
//   bun scripts/ci-scope.ts areas <base> [head]   which checks to run (GITHUB_OUTPUT lines)
//   bun scripts/ci-scope.ts cargo <base> [head]   which crates to check (GITHUB_ENV lines)
// An empty, all-zero or unknown <base> means "check everything". So does a
// changed path that no list below names: add it to the list that reads it.

export interface Areas {
  /** Run every check: the CI wiring itself changed, or the diff is unknown. */
  full: boolean;
  rust: boolean;
  ts: boolean;
  /**
   * A TypeScript change outside source files (dependencies, compiler or tool
   * config), so every bun test runs, not only the ones the change reaches.
   */
  tsGlobal: boolean;
  /** Licences, advisories and sources of Rust dependencies (cargo deny). */
  supply: boolean;
  shell: boolean;
  workflows: boolean;
}

export interface Crate {
  name: string;
  /** Directory relative to the workspace root, without a trailing slash. */
  dir: string;
  /** Workspace crates this crate depends on, any dependency kind. */
  deps: string[];
  /** Has a library target, so `cargo test --doc` applies. */
  hasLib: boolean;
}

/** Files that change what every check does. Anything here runs everything. */
const FULL = [
  /^\.github\/workflows\/ci\.yml$/,
  /^\.github\/actions\//,
  /^scripts\/check\.sh$/,
  /^scripts\/ci-scope\.ts$/,
  /^\.gitattributes$/,
];

/** Rust files outside any crate directory that affect every crate. */
const RUST_GLOBAL = [
  /^Cargo\.(toml|lock)$/,
  /^rust-toolchain(\.toml)?$/,
  /^\.?(rustfmt|clippy)\.toml$/,
  /^\.cargo\//,
  /^\.config\/nextest\.toml$/,
  // Fixtures and the end-to-end harness, which any crate's tests may read.
  /^tests\//,
];

/** Rust code lives in these workspace member roots. */
const RUST_DIRS = /^(app|crates)\//;

const TS = [
  /^(ui|inject|tests)\//,
  /^scripts\/.+\.ts$/,
  /^(package\.json|bun\.lock|bunfig\.toml|\.bun-version|\.editorconfig)$/,
  /^biome\.jsonc?$/,
  /^\.prettierrc(\..+)?$/,
  /(^|\/)tsconfig(\..+)?\.json$/,
  // mado-types generates the TypeScript in ui/src/generated/.
  /^crates\/mado-types\//,
];

const TS_GLOBAL = [
  /^tests\//,
  /(^|\/)package\.json$/,
  /^(bun\.lock|bunfig\.toml|\.bun-version|\.editorconfig)$/,
  /^biome\.jsonc?$/,
  /^\.prettierrc(\..+)?$/,
  /(^|\/)tsconfig(\..+)?\.json$/,
  /(^|\/)(vite|svelte)\.config\.[cm]?[jt]s$/,
];

const SUPPLY = [/(^|\/)Cargo\.(toml|lock)$/, /^(deny|about)\.toml$/, /^rust-toolchain(\.toml)?$/];

const SHELL = [/\.(sh|bash)$/];

const WORKFLOWS = [/^\.github\/(workflows|actions)\//];

/**
 * Files no check reads: docs, community files and personal tool config. A
 * Markdown file that code reads (crate docs pulled in with include_str!) goes
 * in the list for that code instead.
 */
const DOCS = [
  /\.md$/,
  /^docs\//,
  /^LICENSE/,
  /^\.gitignore$/,
  /^\.github\/(ISSUE_TEMPLATE\/|CODEOWNERS$|dependabot\.yml$|VOUCHED\.td$|labeler\.yml$)/,
  /^\.agents\//,
  /^bacon\.toml$/,
];

/** Every list above. A path in none of them runs everything. */
const LISTED = [
  ...FULL,
  RUST_DIRS,
  ...RUST_GLOBAL,
  ...TS,
  ...TS_GLOBAL,
  ...SUPPLY,
  ...SHELL,
  ...WORKFLOWS,
  ...DOCS,
];

const any = (file: string, patterns: RegExp[]) => patterns.some((p) => p.test(file));

export const EVERYTHING: Areas = {
  full: true,
  rust: true,
  ts: true,
  tsGlobal: true,
  supply: true,
  shell: true,
  workflows: true,
};

/** Changed paths no list names. */
export function unlisted(files: readonly string[]): string[] {
  return files.filter((f) => !any(f, LISTED));
}

export function classify(files: readonly string[]): Areas {
  // An unlisted path costs a full run, never a skipped check.
  if (files.some((f) => any(f, FULL)) || unlisted(files).length > 0) return EVERYTHING;
  return {
    full: false,
    rust: files.some(
      (f) => RUST_DIRS.test(f) || any(f, RUST_GLOBAL) || f === "scripts/check-deps.ts",
    ),
    ts: files.some((f) => any(f, TS)),
    tsGlobal: files.some((f) => any(f, TS_GLOBAL)),
    supply: files.some((f) => any(f, SUPPLY)),
    shell: files.some((f) => any(f, SHELL)),
    workflows: files.some((f) => any(f, WORKFLOWS)),
  };
}

/**
 * The crates a change can break: the crates whose files changed, plus every
 * workspace crate that depends on them, directly or not. "all" when a change
 * affects the whole workspace or no crate can be named.
 */
export function affectedCrates(
  files: readonly string[],
  crates: readonly Crate[],
): string[] | "all" {
  if (files.some((f) => any(f, FULL) || any(f, RUST_GLOBAL)) || unlisted(files).length > 0) {
    return "all";
  }
  // Longest directory first, so a nested crate wins over its parent.
  const byDir = [...crates].sort((a, b) => b.dir.length - a.dir.length);
  const touched = new Set<string>();
  for (const file of files) {
    const owner = byDir.find((c) => c.dir === "" || file.startsWith(`${c.dir}/`));
    if (owner) touched.add(owner.name);
  }
  if (touched.size === 0) return "all";

  const dependents = new Map<string, string[]>();
  for (const crate of crates) {
    for (const dep of crate.deps) {
      dependents.set(dep, [...(dependents.get(dep) ?? []), crate.name]);
    }
  }
  const affected = new Set<string>();
  const queue = [...touched];
  for (let name = queue.pop(); name !== undefined; name = queue.pop()) {
    if (affected.has(name)) continue;
    affected.add(name);
    queue.push(...(dependents.get(name) ?? []));
  }
  return [...affected].sort();
}

/** Cargo package names. check.sh pastes the selectors into its commands. */
const CRATE_NAME = /^[A-Za-z0-9_-]+$/;

/** Cargo package selectors for check.sh: all crates, or `-p` per crate. */
export function cargoArgs(affected: string[] | "all", crates: readonly Crate[]) {
  if (affected === "all" || !affected.every((n) => CRATE_NAME.test(n))) {
    return { packages: "--workspace", docPackages: "--workspace" };
  }
  const libs = new Set(crates.filter((c) => c.hasLib).map((c) => c.name));
  const select = (names: string[]) => names.map((n) => `-p ${n}`).join(" ");
  return { packages: select(affected), docPackages: select(affected.filter((n) => libs.has(n))) };
}

async function run(cmd: string[]): Promise<{ ok: boolean; out: string }> {
  const proc = Bun.spawn(cmd, { stdout: "pipe", stderr: "ignore" });
  const out = await new Response(proc.stdout).text();
  return { ok: (await proc.exited) === 0, out };
}

/** Files changed between two commits, or undefined when the diff is unknown. */
async function changedFiles(base: string | undefined, head: string): Promise<string[] | undefined> {
  if (!base || /^0+$/.test(base)) return undefined;
  const diff = await run(["git", "diff", "--name-only", "--no-renames", base, head]);
  if (!diff.ok) return undefined;
  return diff.out.split("\n").filter((line) => line !== "");
}

interface Metadata {
  workspace_root: string;
  packages: {
    name: string;
    manifest_path: string;
    dependencies: { name: string }[];
    targets: { kind: string[] }[];
  }[];
}

async function workspaceCrates(): Promise<Crate[]> {
  const meta = await run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"]);
  if (!meta.ok) throw new Error("cargo metadata failed");
  const { workspace_root: root, packages } = JSON.parse(meta.out) as Metadata;
  const names = new Set(packages.map((p) => p.name));
  return packages.map((p) => ({
    name: p.name,
    dir: p.manifest_path.slice(root.length + 1, -"Cargo.toml".length - 1).replaceAll("\\", "/"),
    deps: p.dependencies.map((d) => d.name).filter((n) => names.has(n)),
    hasLib: p.targets.some((t) => t.kind.some((k) => ["lib", "rlib", "proc-macro"].includes(k))),
  }));
}

if (import.meta.main) {
  const [mode, base, head = "HEAD"] = process.argv.slice(2);
  const files = await changedFiles(base, head);
  if (mode === "areas") {
    const areas = files ? classify(files) : EVERYTHING;
    for (const file of unlisted(files ?? [])) {
      console.error(`no list in scripts/ci-scope.ts names ${file}: checking everything`);
    }
    for (const [key, value] of Object.entries(areas)) console.log(`${key}=${value}`);
    console.error(files ? `${files.length} changed file(s)` : "diff unknown: checking everything");
  } else if (mode === "cargo") {
    const crates = await workspaceCrates();
    const { packages, docPackages } = cargoArgs(
      files ? affectedCrates(files, crates) : "all",
      crates,
    );
    console.log(`CHECK_CARGO_PACKAGES=${packages}`);
    console.log(`CHECK_CARGO_DOC_PACKAGES=${docPackages}`);
    console.error(`cargo scope: ${packages}`);
  } else {
    console.error("usage: bun scripts/ci-scope.ts <areas|cargo> <base> [head]");
    process.exit(2);
  }
}
