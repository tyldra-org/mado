// Bundles each page script in src/ into one self-contained IIFE file in the
// folder given as the first argument, or dist/. crates/mado-engine/build.rs
// runs it into cargo's build folder, which the app embeds
// (mado-docs/spec/02-engine.md section 5).
import { readdir } from "node:fs/promises";
import { join } from "node:path";

const srcDir = join(import.meta.dir, "src");
const outDir = process.argv[2] ?? join(import.meta.dir, "dist");

const entries = (await readdir(srcDir))
  .filter((name) => name.endsWith(".ts") && !name.endsWith(".test.ts"))
  .map((name) => join(srcDir, name));

const result = await Bun.build({
  entrypoints: entries,
  outdir: outDir,
  root: srcDir,
  target: "browser",
  format: "iife",
  minify: true,
});

if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}
console.log(`Built ${result.outputs.length} page script(s) into ${outDir}`);
