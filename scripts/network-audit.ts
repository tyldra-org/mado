// The network audit (mado-docs/spec/08-safety-privacy.md section 11, 16
// section 1): runs the end-to-end suite with every connection Mado makes,
// its own and its pages', through the fixture server's recording proxy, then
// fails on any host outside tests/network-allowlist.json.
//
//   MADO_E2E_BIN=target/debug/mado-app MADO_FIXTURE_BIN=target/debug/mado-fixture-server \
//     bun scripts/network-audit.ts
//
// The app must be built with the test-hooks feature (MADO_TEST_PROXY).
import { mkdtemp, readFile } from "node:fs/promises";
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
  const tests = spawn(["bun", "test", "tests/e2e"], {
    cwd: root,
    env: { ...env, MADO_AUDIT_PROXY: address },
    stdout: "inherit",
    stderr: "inherit",
  });
  const code = await tests.exited;
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
