// Works out what a release run publishes (spec 15 §7, §9): a tag v1.2.3 is
// stable, v1.2.3-beta.N is beta, and any other run is a nightly of the
// version in app/tauri.conf.json.
//
//   bun scripts/release/plan.ts        (in CI: writes GITHUB_OUTPUT lines)
//
// Nightly versions are <next>-nightly.<yyyymmdd>.<run>, so a manual run on
// the same day still sorts after the scheduled one.
import { appendFile, readFile } from "node:fs/promises";

export type ChannelName = "stable" | "beta" | "nightly";

export interface Plan {
  version: string;
  channel: ChannelName;
  prerelease: boolean;
}

export function plan(input: { tag?: string; base: string; date: Date; run: number }): Plan {
  const { tag, base, date, run } = input;
  if (tag === undefined) {
    const day = date.toISOString().slice(0, 10).replaceAll("-", "");
    return { version: `${base}-nightly.${day}.${run}`, channel: "nightly", prerelease: true };
  }
  const version = tag.replace(/^v/, "");
  if (version === base) return { version, channel: "stable", prerelease: false };
  if (/^\d+$/.test(version.slice(`${base}-beta.`.length)) && version.startsWith(`${base}-beta.`))
    return { version, channel: "beta", prerelease: true };
  throw new Error(`tag ${tag} does not match app/tauri.conf.json version ${base}`);
}

if (import.meta.main) {
  const env = process.env;
  const config = JSON.parse(await readFile("app/tauri.conf.json", "utf8")) as { version: string };
  const result = plan({
    tag: env.GITHUB_REF_TYPE === "tag" ? env.GITHUB_REF_NAME : undefined,
    base: config.version,
    date: new Date(),
    run: Number(env.GITHUB_RUN_NUMBER ?? 0),
  });
  const lines = Object.entries(result)
    .map(([k, v]) => `${k}=${v}\n`)
    .join("");
  if (env.GITHUB_OUTPUT) await appendFile(env.GITHUB_OUTPUT, lines);
  process.stdout.write(lines);
}
