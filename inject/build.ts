// Bundles each page script in src/ into one self-contained IIFE file in
// app/resources/inject/, which the app embeds (mado-docs/spec/02-engine.md section 5).
import { readdir } from "node:fs/promises";
import { join } from "node:path";

const srcDir = join(import.meta.dir, "src");
const outDir = join(import.meta.dir, "..", "app", "resources", "inject");

const entries = (await readdir(srcDir))
  .filter((name) => name.endsWith(".ts") && !name.endsWith(".test.ts"))
  .map((name) => join(srcDir, name));

const result = await Bun.build({
  entrypoints: entries,
  outdir: outDir,
  target: "browser",
  format: "iife",
  minify: true,
});

if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}
console.log(`Built ${result.outputs.length} page script(s) into ${outDir}`);
