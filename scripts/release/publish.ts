// Publishes what macos.sh wrote to <dist> (spec 15 §5, §6): one GitHub
// release with the DMG, the updater archive, its signature and SHA-256 sums,
// then the channel file the updater reads, at rollout 1.
//
//   bun scripts/release/publish.ts <dist> <version> <channel> <prerelease> [--dry-run]
//
// --dry-run prints the gh commands instead of running them.
import { mkdtemp, readdir, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { $ } from "bun";
import { channel } from "./channel";

const REPO = process.env.GITHUB_REPOSITORY ?? "tyldra-org/mado";

if (import.meta.main) {
  const args = process.argv.slice(2);
  const dry = args.includes("--dry-run");
  const [dist, version, name, prerelease] = args.filter((a) => a !== "--dry-run");
  if (!dist || !version || !name || !prerelease) {
    console.error("usage: publish.ts <dist> <version> <channel> <prerelease> [--dry-run]");
    process.exit(2);
  }
  const gh = async (...argv: string[]) => {
    if (dry) console.log(["gh", ...argv].join(" "));
    else await $`gh ${argv}`;
  };

  const files = (await readdir(dist)).map((f) => join(dist, f));
  const tag = `v${version}`;
  await gh(
    "release",
    "create",
    tag,
    ...files,
    "--repo",
    REPO,
    ...(process.env.GITHUB_SHA ? ["--target", process.env.GITHUB_SHA] : []),
    "--title",
    `Mado ${version}`,
    "--notes",
    `Mado ${version}. Unsigned by Apple: see Privacy & Security, Open Anyway.`,
    ...(prerelease === "true" ? ["--prerelease"] : []),
  );

  const doc = channel(version, 1, false, {
    "darwin-aarch64": await readFile(join(dist, "Mado_aarch64.app.tar.gz.sig"), "utf8"),
  });
  const out = join(await mkdtemp(join(tmpdir(), "mado-channel-")), `${name}.json`);
  await writeFile(out, `${JSON.stringify(doc, null, 2)}\n`);
  const exists =
    dry || (await $`gh release view channels --repo ${REPO}`.quiet().nothrow()).exitCode === 0;
  if (!exists) {
    await gh(
      "release",
      "create",
      "channels",
      "--repo",
      REPO,
      "--prerelease",
      "--title",
      "Update channels",
      "--notes",
      "The files Mado's updater reads (spec 15 §5). Not a release.",
    );
  }
  await gh("release", "upload", "channels", out, "--repo", REPO, "--clobber");
}
