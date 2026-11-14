// Writes app/resources/credits.json: every bundled Rust crate and
// JavaScript package with its licence, for mado://credits (spec 15 §8).
// Fails when a licence is not on the allow-list in about.toml.
//
//   bun scripts/credits.ts
import { mkdir, readFile, realpath, stat, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { $ } from "bun";

export interface Credit {
  name: string;
  version: string;
  license: string;
  source: "rust" | "javascript";
}

const root = join(import.meta.dir, "..");

/** The SPDX identifiers about.toml accepts. */
export async function allowed(): Promise<Set<string>> {
  const text = await readFile(join(root, "about.toml"), "utf8");
  const list = text.match(/accepted\s*=\s*\[([\s\S]*?)\]/)?.[1] ?? "";
  return new Set([...list.matchAll(/"([^"]+)"/g)].map((m) => m[1] ?? ""));
}

/** True when an SPDX expression is satisfied by allowed licences. */
export function permitted(expression: string, ok: Set<string>): boolean {
  const expr = expression.replace(/[()]/g, " ").trim();
  if (expr === "") return false;
  return expr
    .split(/\s+OR\s+/)
    .some((alt) => alt.split(/\s+AND\s+/).every((id) => ok.has(id.trim())));
}

async function rust(): Promise<Credit[]> {
  // Only what the app links: cargo-about rejects other licences itself.
  const out = await $`cargo about generate --format json -m ${join(root, "app/Cargo.toml")}`
    .cwd(root)
    .quiet()
    .text();
  const report = JSON.parse(out) as {
    licenses: { id: string; used_by: { crate: { name: string; version: string } }[] }[];
  };
  const credits = new Map<string, Credit>();
  for (const license of report.licenses) {
    for (const { crate } of license.used_by) {
      if (crate.name.startsWith("mado-")) continue;
      const key = `${crate.name}@${crate.version}`;
      const known = credits.get(key);
      credits.set(key, {
        name: crate.name,
        version: crate.version,
        license: known ? `${known.license} AND ${license.id}` : license.id,
        source: "rust",
      });
    }
  }
  return [...credits.values()];
}

/** The folder of package `name` as Node resolves it from `from`. */
async function locate(name: string, from: string): Promise<string> {
  for (let dir = from; ; dir = dirname(dir)) {
    const candidate = join(dir, "node_modules", name);
    if (await stat(join(candidate, "package.json")).catch(() => null)) return realpath(candidate);
    if (dirname(dir) === dir) throw new Error(`${name} is not installed (from ${from})`);
  }
}

/** Production dependencies of ui/, and theirs, from node_modules. */
async function javascript(ok: Set<string>): Promise<Credit[]> {
  const seen = new Map<string, Credit>();
  const queue: [string, string][] = [[join(root, "ui"), ""]];
  for (let next = queue.pop(); next !== undefined; next = queue.pop()) {
    const [dir, name] = next;
    const pkg = JSON.parse(await readFile(join(dir, "package.json"), "utf8")) as {
      version: string;
      license?: string;
      dependencies?: Record<string, string>;
    };
    if (name !== "") {
      const key = `${name}@${pkg.version}`;
      if (seen.has(key)) continue;
      const license = pkg.license ?? "UNKNOWN";
      if (!permitted(license, ok)) {
        throw new Error(`${key}: licence ${license} is not on the allow-list in about.toml`);
      }
      seen.set(key, { name, version: pkg.version, license, source: "javascript" });
    }
    for (const dep of Object.keys(pkg.dependencies ?? {}))
      queue.push([await locate(dep, dir), dep]);
  }
  return [...seen.values()];
}

if (import.meta.main) {
  const ok = await allowed();
  const credits = [...(await rust()), ...(await javascript(ok))].sort((a, b) =>
    a.name.localeCompare(b.name),
  );
  await mkdir(join(root, "app/resources"), { recursive: true });
  await writeFile(
    join(root, "app/resources/credits.json"),
    `${JSON.stringify(credits, null, 2)}\n`,
  );
  console.log(`credits: ${credits.length} packages`);
}
