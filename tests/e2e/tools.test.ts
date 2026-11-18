// The agent tools on WebKit (spec 13 §8, roadmap M5-T09): each reads or
// acts on a fixture page with trusted input and answers what changed.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { join } from "node:path";
import { type Subprocess, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;

type Snap = { tab: string; title: string; url: string; snapshot: string };
type Changed = { tab: string; changes: { added: string[]; removed: string[] }; covered?: string };

run("agent tools", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;
  let tab = "";

  beforeAll(async () => {
    fixtures = spawn([fixtureBin as string, "--pages", "tests/pages"], {
      stdout: "pipe",
      stderr: "inherit",
    });
    const reader = fixtures.stdout.getReader();
    let text = "";
    while (!text.includes("\n")) {
      const { value, done } = await reader.read();
      if (done) break;
      text += new TextDecoder().decode(value);
    }
    const proxy = /proxy=(\S+)/.exec(text)?.[1] ?? "";
    mado = await Mado.create();
    await mado.start();
    await mado.control.call("settings.set", { key: "proxy.mode", value: "manual" });
    await mado.control.call("settings.set", { key: "proxy.http", value: proxy });
    await mado.control.call("window.new", { private: false, urls: [] });
    // Uploads and changes to Mado ask first (spec 13 §9); the person here
    // allows each card once, and the safety tests cover the asking.
    void approve();
  });

  let stopped = false;
  const approve = async () => {
    while (!stopped) {
      try {
        const s = (
          await mado.control.call<{
            state: { prompts: Record<string, { id: string; type: string }> };
          }>("state.get")
        ).state;
        for (const c of Object.values(s.prompts).filter((p) => p.type === "aiApproval"))
          await mado.control.call("test.asUser", {
            command: { type: "ai.approval.respond", promptId: c.id, answer: "once" },
          });
      } catch {}
      await Bun.sleep(50);
    }
  };

  afterAll(async () => {
    stopped = true;
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
  });

  const tool = <T>(name: string, args: Record<string, unknown>) =>
    mado.control.call<T>(`tool.${name}`, args);
  const ref = (s: Snap, needle: string) => {
    const line = s.snapshot.split("\n").find((l) => l.includes(needle)) ?? "";
    return /\b(e\d+)\b/.exec(line)?.[1] ?? "";
  };
  const log = async () => (await tool<{ text: string }>("text", { tab })).text;

  test("open, snapshot and text read the page", async () => {
    const s = await tool<Snap>("open", { url: "http://pages.mado.test/ai/tools.html" });
    tab = s.tab;
    expect(s.title).toBe("Tools");
    expect(s.snapshot).toContain('<untrusted source="http://pages.mado.test">');
    expect(s.snapshot).toMatch(/e\d+ button "Add to cart"/);
    const interactive = await tool<Snap>("snapshot", { tab, interactive: true });
    expect(interactive.snapshot).not.toContain("0 items");
    expect((await tool<{ text: string }>("text", { tab })).text).toContain("0 items");
    const shot = await tool<{ image: { mimeType: string; data: string } }>("screenshot", { tab });
    expect(shot.image.data.length).toBeGreaterThan(1000);
  });

  test("click is trusted and answers what changed; a covered element is named", async () => {
    const s = await tool<Snap>("snapshot", { tab });
    const r = await tool<Changed>("click", { tab, ref: ref(s, '"Add to cart"') });
    expect(r.changes.added.join("\n")).toContain("1 item");
    expect(await log()).toContain("click:true");
    const covered = await tool<Changed>("click", { tab, ref: ref(s, '"Covered"') });
    expect(covered.covered).toContain("Cookie banner");
  });

  test("type, press, choose, hover, scroll and drag", async () => {
    const s = await tool<Snap>("snapshot", { tab });
    await tool("type", { tab, ref: ref(s, 'textbox "Name"'), text: "Ada", submit: true });
    const after = await tool<Snap>("snapshot", { tab });
    expect(after.snapshot).toContain('value="Ada"');
    expect(await log()).toContain("input:true");
    expect(await log()).toContain("enter:true");
    await tool("choose", { tab, ref: ref(s, "combobox"), value: "Large" });
    expect(await log()).toContain("size:l");
    await tool("hover", { tab, ref: ref(s, '"Hover me"') });
    expect(await log()).toContain("hovered");
    await tool("scroll", { tab, dy: 50 });
    await tool("drag", { tab, from: ref(s, '"Drag source"'), to: ref(s, '"Drop target"') });
    expect(await log()).toContain("dropped");
    await tool("press", { tab, keys: "Tab" });
  });

  test("run, fetch, upload and download", async () => {
    const ran = await tool<{ added: number; title: string }>("run", {
      code: `const s = await snapshot({ tab: "${tab}" });
        const line = s.snapshot.split("\\n").find((l) => l.includes("Add to cart"));
        const r = line.match(/e\\d+/)[0];
        const c = await click({ tab: "${tab}", ref: r });
        return { added: c.changes.added.length, title: s.title };`,
    });
    expect(ran.title).toBe("Tools");
    expect(ran.added).toBeGreaterThan(0);
    const fetched = await tool<{ status: number; body: string }>("fetch", {
      url: "http://pages.mado.test/ai/report.txt",
    });
    expect(fetched.status).toBe(200);
    expect(fetched.body).toContain("Quarterly report");
    const file = join(mado.dir, "upload.txt");
    await Bun.write(file, "hello");
    const s = await tool<Snap>("snapshot", { tab });
    await tool("upload", { tab, ref: ref(s, '"Attachment"'), files: [file] });
    expect(await log()).toContain("file:upload.txt");
    const d = await tool<{ started: boolean }>("download", {
      tab,
      url: "http://pages.mado.test/ai/report.txt",
    });
    expect(d.started).toBe(true);
  });

  test("wait, navigate, native actions with undo, bookmarks and close", async () => {
    const w = await tool<Snap>("wait", { tab, for: "Add to cart", timeoutMs: 2000 });
    expect(w.snapshot).toContain("Add to cart");
    const back = await tool<Snap>("navigate", {
      tab,
      url: "http://pages.mado.test/reader/article.html",
    });
    expect(back.title).toContain("Fixture story");
    const ctx = await tool<{ tabs: { tab: string }[] }>("native_context", {});
    expect(ctx.tabs.some((t) => t.tab === tab)).toBe(true);
    const actions = await tool<{ action: string }[]>("native_actions", {});
    expect(actions.some((a) => a.action === "tab.new")).toBe(true);
    expect(actions.some((a) => a.action.startsWith("vault."))).toBe(false);
    const applied = await tool<{ undoToken: string }>("native_apply", {
      action: "settings.set",
      args: { key: "appearance.theme", value: "dark", token: null },
    });
    await tool("native_undo", { token: applied.undoToken });
    await tool("bookmark_add", { url: "http://pages.mado.test/ai/tools.html", title: "Tools" });
    expect(JSON.stringify(await tool("bookmarks", { query: "Tools" }))).toContain("tools.html");
    await tool("close", { tab });
  });
});
