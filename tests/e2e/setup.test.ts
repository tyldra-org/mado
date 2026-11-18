// \`mado setup\` and \`mado remove\` with the real CLI (spec 14 §5, roadmap
// M5-T16; 14 §6 check 5) in a throwaway home folder.
import { afterAll, describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawn } from "bun";

const cli = process.env.MADO_CLI_BIN ?? "target/debug/mado";
const run = existsSync(cli) ? describe : describe.skip;

run("mado setup", () => {
  let dir = "";
  afterAll(async () => {
    if (dir) await rm(dir, { recursive: true, force: true });
  });
  const mado = async (...args: string[]) => {
    const p = spawn([cli, ...args], {
      env: { ...process.env, HOME: join(dir, "home"), MADO_DATA_DIR: join(dir, "data") },
      stdout: "pipe",
      stderr: "pipe",
    });
    return { code: await p.exited, out: await new Response(p.stdout).text() };
  };

  test("setup codex then remove codex leaves config.toml byte-identical (14 §6 check 5)", async () => {
    dir = await mkdtemp(join(tmpdir(), "mado-setup-"));
    const file = join(dir, "home", ".codex", "config.toml");
    await mkdir(join(dir, "home", ".codex"), { recursive: true });
    const original = '# mine\nmodel = "gpt-5"  # keep\n\n[mcp_servers.other]\ncommand = "x"\n';
    await writeFile(file, original);
    expect((await mado("setup", "codex")).code).toBe(0);
    expect(await readFile(file, "utf8")).toContain("[mcp_servers.mado]");
    const st = JSON.parse((await mado("setup", "--json")).out) as {
      targets: { target: string; setUp: boolean }[];
    };
    expect(st.targets.find((t) => t.target === "codex")?.setUp).toBe(true);
    expect((await mado("remove", "codex")).code).toBe(0);
    expect(await readFile(file, "utf8")).toBe(original);
    expect((await mado("setup", "nowhere")).code).toBe(2);
  });
});
