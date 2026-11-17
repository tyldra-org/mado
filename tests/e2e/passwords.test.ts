// Mado Passwords on WebKit (spec 11 §12 checks 1 and 2, roadmap M4-T03):
// a click on a visible field opens the fill menu, the fill reaches only the
// visible fields of the top frame, and a fill without a click is refused.
import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { type Subprocess, spawn } from "bun";
import { bin, Mado } from "./harness";

const fixtureBin = process.env.MADO_FIXTURE_BIN;
const run = bin && fixtureBin ? describe : describe.skip;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(
  what: string,
  probe: () => Promise<T | undefined>,
  ms = 30_000,
): Promise<T> {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await probe();
    if (v !== undefined) return v;
    await sleep(150);
  }
  throw new Error(`timed out waiting for ${what}`);
}

type Menu = {
  type: string;
  id: string;
  tabId: string;
  formKey: string;
  loading: boolean;
  items: { itemRef: string }[];
};
type State = {
  prompts: Record<string, Menu>;
  tabs: Record<string, { title: string; url: string }>;
  windows: Record<string, { tabOrder: string[] }>;
};

run("passwords", () => {
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
  const evalIn = async <T>(tabId: string, source: string): Promise<T> =>
    JSON.parse(await mado.control.call<string>("test.eval", { tabId, source })) as T;

  test("the fill reaches only the visible fields of the top frame (11 §12 checks 1, 2)", async () => {
    await asUser({ type: "vault.setup" });
    await asUser({
      type: "vault.save",
      item: {
        kind: "login",
        title: "",
        urls: ["http://pages.mado.test/"],
        username: "me@example.com",
        password: "hunter2",
        passwordHistory: [],
      },
    });
    await asUser({
      type: "vault.save",
      item: {
        kind: "login",
        title: "",
        urls: ["https://elsewhere.example/"],
        username: "other",
        password: "nope",
        passwordHistory: [],
      },
    });
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["http://pages.mado.test/passwords/login.html"],
    });
    const tabId = await until("the tab", async () => {
      const s = await state();
      const t = s.windows[windowId]?.tabOrder[0];
      return t && s.tabs[t]?.title === "Sign in" ? t : undefined;
    });
    await sleep(500);
    const rect = await evalIn<{ x: number; y: number }>(
      tabId,
      `const r = document.getElementById("user").getBoundingClientRect();
       return JSON.stringify({ x: r.left + r.width / 2, y: r.top + r.height / 2 });`,
    );
    // Check 2: a fill with no click is refused.
    await sleep(1200);
    await expect(
      asUser({ type: "vault.fill", tabId, formKey: "00", itemRef: "mado:x" }),
    ).rejects.toThrow();
    await mado.control.call("test.click", { tabId, x: rect.x, y: rect.y });
    const menu = await until("the fill menu", async () =>
      Object.values((await state()).prompts).find((p) => p.type === "fillMenu" && !p.loading),
    );
    expect(menu.items.length).toBe(1);
    // Choosing the item is a click in Mado's window; the field's click
    // stands in for it here.
    await mado.control.call("test.click", { tabId, x: rect.x, y: rect.y });
    await asUser({
      type: "vault.fill",
      tabId,
      formKey: menu.formKey,
      itemRef: menu.items[0]?.itemRef,
    });
    const values = await until("the values", async () => {
      const v = await evalIn<Record<string, string>>(
        tabId,
        `return JSON.stringify(Object.fromEntries(["user", "pass", "hidden", "offscreen"].map((id) => [id, document.getElementById(id).value])));`,
      );
      return v.user ? v : undefined;
    });
    expect(values).toEqual({ user: "me@example.com", pass: "hunter2", hidden: "", offscreen: "" });
    // Close any menu the second click opened, then a click on the login in
    // the frame of another origin offers nothing.
    for (const p of Object.values((await state()).prompts)) {
      if (p.type === "fillMenu") await asUser({ type: "prompt.dismiss", id: p.id });
    }
    const frame = await evalIn<{ x: number; y: number }>(
      tabId,
      `const r = document.querySelector("iframe").getBoundingClientRect();
       return JSON.stringify({ x: r.left + 20, y: r.top + 18 });`,
    );
    await mado.control.call("test.click", { tabId, x: frame.x, y: frame.y });
    await sleep(800);
    expect(Object.values((await state()).prompts).some((p) => p.type === "fillMenu")).toBe(false);
  }, 60_000);

  test("a changed password that signs in is offered as an update (11 §5)", async () => {
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["http://pages.mado.test/passwords/login.html"],
    });
    const tabId = await until("the tab", async () => {
      const s = await state();
      const t = s.windows[windowId]?.tabOrder[0];
      return t && s.tabs[t]?.title === "Sign in" ? t : undefined;
    });
    await sleep(500);
    const button = await evalIn<{ x: number; y: number }>(
      tabId,
      `document.getElementById("user").value = "me@example.com";
       document.getElementById("pass").value = "a-new-password";
       const r = document.querySelector("button").getBoundingClientRect();
       return JSON.stringify({ x: r.left + r.width / 2, y: r.top + r.height / 2 });`,
    );
    await mado.control.call("test.click", { tabId, x: button.x, y: button.y });
    type Save = { type: string; id: string; update: boolean; site: string };
    const prompt = await until("the update prompt", async () =>
      Object.values((await state()).prompts as unknown as Record<string, Save>).find(
        (p) => p.type === "savePassword",
      ),
    );
    expect(prompt.update).toBe(true);
    expect(prompt.site).toBe("mado.test");
    await asUser({ type: "vault.saveRespond", promptId: prompt.id, action: "save" });
    const items = await until("updated", async () => {
      const list = (await asUser({ type: "vault.query", site: "pages.mado.test" })) as {
        id: string;
      }[];
      const got = (await asUser({ type: "vault.get", id: list[0]?.id ?? "" })) as {
        item: { password: string };
      };
      return got.item.password === "a-new-password" ? got : undefined;
    });
    expect(items.item.password).toBe("a-new-password");
  }, 60_000);

  test("a QR code on the page adds a two-factor code (11 §7)", async () => {
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["http://pages.mado.test/passwords/totp.html"],
    });
    const tabId = await until("the tab", async () => {
      const s = await state();
      const t = s.windows[windowId]?.tabOrder[0];
      return t && s.tabs[t]?.title === "Two-factor setup" ? t : undefined;
    });
    await sleep(800);
    const scan = (await asUser({ type: "vault.scanQr", tabId })) as {
      scanId: string;
      issuer: string;
      account: string;
    };
    expect(scan.issuer).toBe("Mado Fixture");
    expect(scan.account).toBe("me@example.com");
    const { id } = (await asUser({ type: "vault.attachTotp", scanId: scan.scanId })) as {
      id: string;
    };
    const got = (await asUser({ type: "vault.get", id })) as {
      item: { kind: string; uri: string };
    };
    expect(got.item.kind).toBe("totp");
    expect(got.item.uri).toContain("secret=JBSWY3DPEHPK3PXP");
  }, 60_000);

  test("a passkey made in Mado signs a sign-in the page verifies (11 §12 check 3)", async () => {
    await asUser({ type: "vault.setup" }).catch(() => {});
    const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
      private: false,
      urls: ["https://pages.mado.test/passwords/passkey.html"],
    });
    const tabId = await until(
      "the tab",
      async () => (await state()).windows[windowId]?.tabOrder[0],
    );
    // The fixture's certificate is not trusted on a developer's Mac: Proceed.
    await until("the certificate page or the fixture", async () => {
      const t = (await state()).tabs[tabId]?.title;
      if (t === "Passkeys") return true;
      if (t && t !== "pending") {
        await mado.control.call("test.errorAction", { tabId, action: "proceed" }).catch(() => {});
      }
      return undefined;
    });
    await sleep(500);
    const at = (id: string) =>
      evalIn<{ x: number; y: number }>(
        tabId,
        `const r = document.getElementById("${id}").getBoundingClientRect();
         return JSON.stringify({ x: r.left + r.width / 2, y: r.top + r.height / 2 });`,
      );
    const data = (key: string) =>
      evalIn<string | null>(tabId, `return JSON.stringify(document.body.dataset.${key} ?? null);`);
    type Sheet = { type: string; id: string; mode: string; accounts: { itemId: string }[] };
    const sheet = (mode: string) =>
      until(`the ${mode} sheet`, async () =>
        Object.values((await state()).prompts as unknown as Record<string, Sheet>).find(
          (p) =>
            p.type === "passkey" && p.mode === mode && (mode === "create" || p.accounts.length > 0),
        ),
      );
    const create = await at("create");
    await mado.control.call("test.click", { tabId, ...create });
    const made = await sheet("create");
    await asUser({ type: "vault.passkeyRespond", promptId: made.id, action: "save" });
    expect(await until("created", async () => (await data("created")) ?? undefined)).toBe(
      "public-key:webauthn.create:true",
    );
    const get = await at("get");
    await mado.control.call("test.click", { tabId, ...get });
    const use = await sheet("get");
    await asUser({
      type: "vault.passkeyRespond",
      promptId: use.id,
      action: "use",
      itemId: use.accounts[0]?.itemId,
    });
    expect(await until("verified", async () => (await data("verified")) ?? undefined)).toBe("yes");
  }, 60_000);

  // The live site, so only with MADO_E2E_NETWORK=1.
  (process.env.MADO_E2E_NETWORK === "1" ? test : test.skip)(
    "webauthn.io registers and signs in with a Mado passkey (11 §12 check 3)",
    async () => {
      await asUser({ type: "vault.setup" }).catch(() => {});
      await mado.control.call("settings.set", { key: "proxy.mode", value: "direct" });
      const { windowId } = await mado.control.call<{ windowId: string }>("window.new", {
        private: false,
        urls: ["https://webauthn.io/"],
      });
      const tabId = await until("webauthn.io", async () => {
        const s = await state();
        const t = s.windows[windowId]?.tabOrder[0];
        return t && s.tabs[t]?.title === "WebAuthn.io" ? t : undefined;
      });
      await sleep(1500);
      const name = `mado${Date.now()}`;
      await evalIn(
        tabId,
        `const i = document.querySelector("#input-email");
         Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(i, "${name}");
         i.dispatchEvent(new Event("input", { bubbles: true }));
         document.querySelector("#register-button").click();
         return "1";`,
      );
      type Sheet = { type: string; id: string; mode: string; accounts: { itemId: string }[] };
      const sheet = (mode: string) =>
        until(`the ${mode} sheet`, async () =>
          Object.values((await state()).prompts as unknown as Record<string, Sheet>).find(
            (p) =>
              p.type === "passkey" &&
              p.mode === mode &&
              (mode === "create" || p.accounts.length > 0),
          ),
        );
      const made = await sheet("create");
      await asUser({ type: "vault.passkeyRespond", promptId: made.id, action: "save" });
      await sleep(3000);
      await evalIn(tabId, `document.querySelector("#login-button").click(); return "1";`);
      const use = await sheet("get");
      await asUser({
        type: "vault.passkeyRespond",
        promptId: use.id,
        action: "use",
        itemId: use.accounts[0]?.itemId,
      });
      const url = await until("logged in", async () => {
        const u = (await state()).tabs[tabId]?.url;
        return u?.includes("/profile") ? u : undefined;
      });
      expect(url).toBe("https://webauthn.io/profile");
    },
    90_000,
  );
});
