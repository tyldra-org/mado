// Fails when a workspace crate depends on another workspace crate it must not.
// Rules come from mado-docs/spec/00-overview.md (crate dependency direction).
// Usage: bun scripts/check-deps.ts

export type Graph = Record<string, string[]>;

const ADAPTERS = ["mado-engine-webkit"];

export function findViolations(graph: Graph): string[] {
  const violations: string[] = [];
  for (const [crate, deps] of Object.entries(graph)) {
    for (const dep of deps) {
      if (dep === "mado-app") {
        violations.push(`${crate} -> mado-app: nothing may depend on the app`);
      }
      if (ADAPTERS.includes(dep) && crate !== "mado-app") {
        violations.push(`${crate} -> ${dep}: only mado-app may depend on an engine adapter`);
      }
      // mado-types-derive is mado-types' own proc-macro (the Patchable derive).
      if (crate === "mado-types" && dep !== "mado-types-derive") {
        violations.push(`mado-types -> ${dep}: mado-types depends only on mado-types-derive`);
      }
      if (crate === "mado-types-derive") {
        violations.push(`mado-types-derive -> ${dep}: the derive depends on no workspace crate`);
      }
      if ((crate === "mado-cli" || crate === "mado-engine") && dep !== "mado-types") {
        violations.push(`${crate} -> ${dep}: ${crate} may depend only on mado-types`);
      }
    }
  }
  return violations;
}

interface Metadata {
  packages: { name: string; dependencies: { name: string }[] }[];
  workspace_members: string[];
}

async function workspaceGraph(): Promise<Graph> {
  const proc = Bun.spawn(["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"], {
    stdout: "pipe",
  });
  const text = await new Response(proc.stdout).text();
  if ((await proc.exited) !== 0) {
    throw new Error("cargo metadata failed");
  }
  const meta = JSON.parse(text) as Metadata;
  const names = new Set(meta.packages.map((p) => p.name));
  const graph: Graph = {};
  for (const pkg of meta.packages) {
    graph[pkg.name] = pkg.dependencies.map((d) => d.name).filter((n) => names.has(n));
  }
  return graph;
}

if (import.meta.main) {
  const violations = findViolations(await workspaceGraph());
  if (violations.length > 0) {
    for (const v of violations) console.error(`dependency rule broken: ${v}`);
    process.exit(1);
  }
  console.log("Crate dependency direction is clean.");
}
