// Extensions end to end (spec 09 §12, roadmap M3-T03): the real Chrome Web
// Store, so it needs the network and runs only with MADO_E2E_NETWORK=1.
// uBlock Origin Lite downloads, verifies with the store's proof, shows the
// install prompt, and Add installs it into the profile; Dark Reader, loaded
// through WKWebExtensionController (M3-T04), turns a white fixture page dark.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { chmod, mkdir, mkdtemp, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Subprocess, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin && process.env.MADO_E2E_NETWORK === "1" ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const UBOL = "ddkjiahejlhfcafbddmgiahcphecmpfh";
const DARK_READER = "eimadpbcbfnmbkopoojfekhnkhdbieeh";

async function until<T>(
  what: string,
  probe: () => Promise<T | undefined>,
  ms = 60_000,
): Promise<T> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe();
    if (v !== undefined) return v;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}`);
}

type Prompt = { type: string; id: string; extensionId: string; name: string };
type State = {
  prompts: Record<string, Prompt>;
  extensions: { installed: { id: string; version: string }[] };
  profile: { id: string };
};

run("extensions from the Chrome Web Store", () => {
  let mado: Mado;
  let fixtures: Subprocess<"ignore", "pipe", "inherit">;

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
    // Fixture pages through the fixture server; the store and Google's
    // download hosts are reached by Mado itself, outside the page proxy.
    await mado.control.call("settings.set", { key: "proxy.mode", value: "manual" });
    await mado.control.call("settings.set", { key: "proxy.http", value: proxy });
  });

  afterAll(async () => {
    await mado?.quit();
    await mado?.remove();
    fixtures?.kill();
  });

  const state = async () => (await mado.control.call<{ state: State }>("state.get")).state;
  const asUser = (command: Record<string, unknown>) =>
    mado.control.call("test.asUser", { command });

  test("uBlock Origin Lite installs through the prompt", async () => {
    await expect(
      mado.control.call("extension.install", { id: UBOL, source: "store" }),
    ).rejects.toThrow();
    await asUser({ type: "extension.install", id: UBOL, source: "store" });
    const prompt = await until("the install prompt", async () =>
      Object.values((await state()).prompts).find((p) => p.type === "extensionInstall"),
    );
    expect(prompt.extensionId).toBe(UBOL);
    await asUser({ type: "extension.confirmInstall", promptId: prompt.id, add: true });
    const installed = await until("installed", async () =>
      (await state()).extensions.installed.find((e) => e.id === UBOL),
    );
    const profile = (await state()).profile.id;
    const manifest = join(
      mado.dir,
      "Profiles",
      profile,
      "Extensions",
      UBOL,
      installed.version,
      "manifest.json",
    );
    expect((await stat(manifest)).isFile()).toBe(true);
  });

  test("Dark Reader loads in WebKit and darkens a white page", async () => {
    await asUser({ type: "extension.install", id: DARK_READER, source: "store" });
    const prompt = await until("the install prompt", async () =>
      Object.values((await state()).prompts).find(
        (p) => p.type === "extensionInstall" && p.extensionId === DARK_READER,
      ),
    );
    await asUser({ type: "extension.confirmInstall", promptId: prompt.id, add: true });
    await until("installed", async () =>
      (await state()).extensions.installed.find((e) => e.id === DARK_READER),
    );
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["http://pages.mado.test/extensions/plain.html"],
    });
    type Tabs = {
      tabs: Record<string, { title: string }>;
      windows: Record<string, { tabOrder: string[] }>;
    };
    const title = await until("the colour", async () => {
      const s = (await mado.control.call<{ state: Tabs }>("state.get")).state;
      const tab = s.windows[windowId]?.tabOrder[0];
      const t = tab ? s.tabs[tab]?.title : undefined;
      return t && t !== "pending" ? t : undefined;
    });
    expect(title).not.toBe("rgb(255, 255, 255)");
  });

  test("each extension's popup opens from the toolbar (M3-T06)", async () => {
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["http://pages.mado.test/extensions/plain.html"],
    });
    type Toolbar = { extensions: { toolbar: { id: string; hasPopup: boolean }[] } };
    for (const id of [UBOL, DARK_READER]) {
      await until(`${id} in the toolbar`, async () =>
        (await mado.control.call<{ state: Toolbar }>("state.get")).state.extensions.toolbar.find(
          (b) => b.id === id && b.hasPopup,
        ),
      );
      await mado.control.call("extension.openPopup", {
        id,
        windowId,
        anchor: { x: 600, y: 10, width: 24, height: 24 },
      });
      const url = await until(`${id}'s popup`, async () => {
        const u = await mado.control.call<string | null>("test.extensionPopup");
        return u?.startsWith("webkit-extension://") ? u : undefined;
      });
      expect(url).toContain("/");
    }
  });

  test("an extension talks to a native host by message and by port (09 §12 check 6)", async () => {
    // Developer mode is protected: authenticate as Mado's window would.
    await mado.control.call("test.fakeAuthSuccess");
    const { token } = (await asUser({ type: "app.authenticate", reason: "test" })) as {
      token: string;
    };
    await asUser({ type: "settings.set", key: "extensions.developerMode", value: true, token });
    await asUser({
      type: "extension.install",
      id: null,
      source: "unpacked",
      path: join(process.cwd(), "tests/extensions/native"),
    });
    const prompt = await until("the install prompt", async () =>
      Object.values((await state()).prompts).find(
        (p) => p.type === "extensionInstall" && p.name === "Mado native messaging fixture",
      ),
    );
    // The host answers {"echo": <message>, "origin": <chrome-extension://id/>}.
    const hostDir = await mkdtemp(join(tmpdir(), "mado-native-"));
    const hostPath = join(hostDir, "echo.py");
    await writeFile(
      hostPath,
      [
        "#!/usr/bin/env python3",
        "import json, struct, sys",
        "origin = sys.argv[1]",
        "while True:",
        "    raw = sys.stdin.buffer.read(4)",
        "    if len(raw) < 4:",
        "        break",
        "    n = struct.unpack('=I', raw)[0]",
        "    msg = json.loads(sys.stdin.buffer.read(n))",
        "    out = json.dumps({'echo': msg, 'origin': origin}).encode()",
        "    sys.stdout.buffer.write(struct.pack('=I', len(out)) + out)",
        "    sys.stdout.buffer.flush()",
        "",
      ].join("\n"),
    );
    await chmod(hostPath, 0o755);
    await mkdir(join(mado.dir, "NativeMessagingHosts"), { recursive: true });
    await writeFile(
      join(mado.dir, "NativeMessagingHosts", "app.mado.test_echo.json"),
      JSON.stringify({
        name: "app.mado.test_echo",
        type: "stdio",
        path: hostPath,
        allowed_origins: [`chrome-extension://${prompt.extensionId}/`],
      }),
    );
    await asUser({ type: "extension.confirmInstall", promptId: prompt.id, add: true });
    type Tabs = { tabs: Record<string, { url: string }> };
    const reported = async (kind: string) =>
      until(`the ${kind} answer`, async () => {
        const s = (await mado.control.call<{ state: Tabs }>("state.get")).state;
        const url = Object.values(s.tabs).find((t) => t.url.includes(`?${kind}=`))?.url;
        return url ? JSON.parse(decodeURIComponent(url.split(`?${kind}=`)[1] ?? "")) : undefined;
      });
    const once = await reported("once");
    expect(once.echo).toEqual({ ping: "once" });
    expect(once.origin).toBe(`chrome-extension://${prompt.extensionId}/`);
    const port = await reported("port");
    expect(port.echo).toEqual({ ping: "port" });
  });

  test("uBlock Origin Lite's overlap prompt turns the blocker off, and removing it turns it back on (09 §12 check 4)", async () => {
    type S = {
      prompts: Record<string, { type: string; id: string; extensionId?: string; default?: string }>;
      settings: { values: Record<string, unknown> };
    };
    const st = async () => (await mado.control.call<{ state: S }>("state.get")).state;
    const prompt = await until("the overlap prompt", async () =>
      Object.values((await st()).prompts).find(
        (p) => p.type === "overlap" && p.extensionId === UBOL,
      ),
    );
    await asUser({ type: "extension.overlapRespond", promptId: prompt.id, choice: prompt.default });
    expect((await st()).settings.values["blocker.enabled"]).toBe(false);
    await asUser({ type: "extension.remove", id: UBOL, deleteData: true });
    expect((await st()).settings.values["blocker.enabled"]).toBe(true);
  });
});
