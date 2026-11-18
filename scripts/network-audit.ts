// The network audit (mado-docs/spec/08-safety-privacy.md section 11, 16
// section 1): runs the end-to-end suite with every connection Mado makes,
// its own and its pages', through the fixture server's recording proxy, then
// fails on any host outside tests/network-allowlist.json.
//
//   MADO_E2E_BIN=target/debug/mado-app MADO_FIXTURE_BIN=target/debug/mado-fixture-server \
//     bun scripts/network-audit.ts
//
// The app must be built with the test-hooks feature (MADO_TEST_PROXY).
import { mkdtemp, readdir, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "bun";

const root = join(import.meta.dir, "..");

interface Allowed {
  pattern: string;
  why: string;
}

/** Whether `host` matches `pattern`: exact, or `*.suffix` for the suffix and below. */
export function matches(host: string, pattern: string): boolean {
  if (pattern.startsWith("*.")) {
    const suffix = pattern.slice(2);
    return host === suffix || host.endsWith(`.${suffix}`);
  }
  return host === pattern;
}

/** The hosts in `recorded` no pattern allows, sorted and once each. */
export function unlisted(recorded: readonly string[], allowed: readonly string[]): string[] {
  const hosts = new Set(recorded.map((h) => h.trim().toLowerCase()).filter((h) => h !== ""));
  return [...hosts].filter((h) => !allowed.some((p) => matches(h, p))).sort();
}

/** The hosts of the bundled search engines' search and suggestion URLs. */
export function engineHosts(source: string): string[] {
  const hosts = new Set<string>();
  for (const m of source.matchAll(/"https:\/\/([a-z0-9.-]+)\/[^"]*\{searchTerms\}/g)) {
    if (m[1]) hosts.add(m[1]);
  }
  return [...hosts].sort();
}

export async function allowlist(): Promise<string[]> {
  const file = JSON.parse(await readFile(join(root, "tests/network-allowlist.json"), "utf8")) as {
    hosts: Allowed[];
  };
  const engines = engineHosts(
    await readFile(join(root, "crates/mado-navigation/src/engines.rs"), "utf8"),
  );
  return [...file.hosts.map((h) => h.pattern), ...engines];
}

async function main(): Promise<number> {
  const fixtureBin = process.env.MADO_FIXTURE_BIN;
  if (!process.env.MADO_E2E_BIN || !fixtureBin) {
    console.error("network-audit: set MADO_E2E_BIN and MADO_FIXTURE_BIN");
    return 2;
  }
  const record = join(await mkdtemp(join(tmpdir(), "mado-audit-")), "hosts.txt");
  // Every fixture server the tests start records into the same file.
  const env = { ...process.env, MADO_NETWORK_RECORD: record };
  const proxy = spawn([fixtureBin, "--pages", join(root, "tests/pages")], {
    env,
    stdout: "pipe",
    stderr: "inherit",
  });
  const reader = proxy.stdout.getReader();
  let text = "";
  while (!text.includes("\n")) {
    const { value, done } = await reader.read();
    if (done) break;
    text += new TextDecoder().decode(value);
  }
  const address = /proxy=(\S+)/.exec(text)?.[1];
  if (!address) {
    console.error("network-audit: the recording proxy did not start");
    proxy.kill();
    return 1;
  }
  // One file at a time, each run again once if it fails: a loaded CI
  // runner sometimes misses a page load by a few seconds, and a release
  // should not stop on that. Files that pass only on the second run are
  // named, so a real flake stays visible.
  const files = (await readdir(join(root, "tests/e2e")))
    .filter((f) => f.endsWith(".test.ts"))
    .sort();
  // The blocker's file is the one that updates the filter lists, so their
  // hosts are recorded without every launch compiling them.
  const run = (file: string) =>
    spawn(["bun", "test", "--timeout", "120000", join("tests/e2e", file)], {
      cwd: root,
      env: {
        ...env,
        MADO_AUDIT_PROXY: address,
        ...(file === "blocker.test.ts" ? { MADO_AUDIT_LISTS: "1" } : {}),
      },
      stdout: "inherit",
      stderr: "inherit",
    }).exited;
  const failed: string[] = [];
  const retried: string[] = [];
  for (const file of files) {
    if ((await run(file)) === 0) continue;
    if ((await run(file)) === 0) retried.push(file);
    else failed.push(file);
  }
  const code = failed.length ? 1 : 0;
  if (retried.length) console.log(`network-audit: passed on the second run: ${retried.join(", ")}`);
  if (failed.length) console.error(`network-audit: failed twice: ${failed.join(", ")}`);
  proxy.kill();
  const recorded = (await readFile(record, "utf8").catch(() => "")).split("\n");
  const bad = unlisted(recorded, await allowlist());
  const seen = unlisted(recorded, []);
  console.log(`network-audit: ${seen.length} hosts contacted: ${seen.join(", ")}`);
  if (bad.length > 0) {
    console.error(`network-audit: hosts not in tests/network-allowlist.json: ${bad.join(", ")}`);
    return 1;
  }
  if (code !== 0) {
    console.error("network-audit: the end-to-end suite failed");
    return code ?? 1;
  }
  return 0;
}

if (import.meta.main) process.exit(await main());
