// M3-T03: the extension install prompt shows what the extension asks for
// and answers extension.confirmInstall.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, test } from "vitest";
import { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import type { State } from "../generated/State";
import MainWindow from "./MainWindow.svelte";

const W = "01a1d7aa-0000-7000-8000-000000000001";
const A = "01a1d7aa-0000-7000-8000-00000000000a";

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

function setup(extra: Partial<Record<string, unknown>> = {}) {
  const state = {
    rev: 0,
    profile: { id: "p", name: "Personal", color: "", avatar: "", isPrivate: false, isGuest: false },
    windows: { [W]: { id: W, layout: "vertical", tabOrder: [A], activeTabId: A, overlay: null } },
    tabs: {
      [A]: {
        id: A,
        windowId: W,
        groupId: null,
        pinned: false,
        title: "Alpha",
        customTitle: null,
        url: "https://alpha.example/",
        displayUrl: { text: "alpha.example", emphasisStart: 0, emphasisEnd: 13, punycode: false },
        security: { level: "secure" },
        favicon: null,
        life: "live",
        audio: { playing: false, muted: false },
      },
    },
    groups: {},
    prompts: {},
    notices: [],
    settings: { values: {}, changed: [] },
    features: {},
    downloads: {},
    ...extra,
  } as unknown as State;
  const seen: Request[] = [];
  let engineChosen = false;
  const transport: Transport = {
    label: () => `ui-${W}`,
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const cmd = request.cmd;
      const ok = (value: unknown): Response =>
        ({ status: "ok", id: request.id, value }) as Response;
      if (cmd.type === "state.get") return ok({ rev: 0, state: structuredClone(state) });
      if (cmd.type === "nav.suggest")
        return ok({
          requestId: cmd.requestId,
          rows: [
            {
              kind: "history",
              input: "https://github.com/",
              title: "GitHub",
              url: "https://github.com/",
              tabId: null,
              score: 1400,
              removable: true,
            },
          ],
          inline: cmd.input.includes(" ") ? null : "hub.com",
        });
      if (cmd.type === "nav.open") return ok(engineChosen ? { tabId: A } : { needsEngine: true });
      if (cmd.type === "nav.searchEngines")
        return ok([{ id: "duckduckgo", name: "DuckDuckGo", active: true }]);
      if (cmd.type === "settings.set") engineChosen = true;
      return ok(null);
    },
  };
  return { store: new CoreStore(transport), seen };
}

async function shown(extra?: Partial<Record<string, unknown>>) {
  const { store, seen } = setup(extra);
  await store.start();
  render(MainWindow, { store, windowId: W });
  await tick();
  return seen;
}

const sent = (seen: Request[], type: string) =>
  seen.filter((r) => r.cmd.type === type).map((r) => r.cmd);

describe("extension install prompt", () => {
  test("it lists the request and Add answers once", async () => {
    const seen = await shown({
      prompts: {
        "01a1d7aa-0000-7000-8000-0000000000e1": {
          type: "extensionInstall",
          id: "01a1d7aa-0000-7000-8000-0000000000e1",
          windowId: W,
          extensionId: "ddkjiahejlhfcafbddmgiahcphecmpfh",
          name: "Test Blocker",
          version: "1.2",
          icon: null,
          permissions: ["storage"],
          hosts: ["<all_urls>"],
          allSites: true,
          optional: [],
          unavailable: ["history"],
          mayNotWork: true,
          olderFormat: false,
          replaces: null,
        },
      },
    });
    const dialog = await screen.findByTestId("extension-install");
    expect(dialog.textContent).toContain("Add “Test Blocker”?");
    expect(dialog.textContent).toContain("all websites");
    expect(dialog.textContent).toContain("history isn't available in Mado yet.");
    expect(dialog.textContent).toContain("Some features may not work.");
    await fireEvent.click(screen.getByText("Add extension"));
    await tick();
    const sent = seen.map((r) => r.cmd).filter((c) => c.type === "extension.confirmInstall");
    expect(sent).toEqual([
      {
        type: "extension.confirmInstall",
        promptId: "01a1d7aa-0000-7000-8000-0000000000e1",
        add: true,
      },
    ]);
  });
});

describe("extension toolbar", () => {
  test("a pinned button opens its popup under itself", async () => {
    const id = "ddkjiahejlhfcafbddmgiahcphecmpfh";
    const seen = await shown({
      extensions: {
        pinned: [id],
        installed: [],
        toolbar: [
          {
            id,
            name: "uBlock Origin Lite",
            iconUrl: "",
            badgeText: "4",
            badgeColor: "",
            title: "uBO Lite",
            hasPopup: true,
            enabled: true,
            siteAccessForActiveTab: "granted",
          },
        ],
      },
    });
    const button = await screen.findByTitle("uBO Lite");
    expect(button.textContent).toContain("4");
    await fireEvent.click(button);
    await tick();
    const popup = sent(seen, "extension.openPopup")[0] as {
      id: string;
      windowId: string;
      anchor: object;
    };
    expect(popup.id).toBe(id);
    expect(popup.windowId).toBe(W);
    expect(popup.anchor).toHaveProperty("width");
  });
});

describe("overlap prompt", () => {
  test("it offers the choices and sends the one picked", async () => {
    const seen = await shown({
      prompts: {
        "01a1d7aa-0000-7000-8000-0000000000f1": {
          type: "overlap",
          id: "01a1d7aa-0000-7000-8000-0000000000f1",
          category: "adBlocker",
          extensionId: "ddkjiahejlhfcafbddmgiahcphecmpfh",
          name: "uBlock Origin Lite",
          choices: ["extensionOnly", "both", "madoOnly"],
          default: "extensionOnly",
        },
      },
    });
    const dialog = await screen.findByTestId("overlap-prompt");
    expect(dialog.textContent).toContain("uBlock Origin Lite blocks ads.");
    await fireEvent.click(screen.getByText("Both"));
    await tick();
    expect(sent(seen, "extension.overlapRespond")).toEqual([
      {
        type: "extension.overlapRespond",
        promptId: "01a1d7aa-0000-7000-8000-0000000000f1",
        choice: "both",
      },
    ]);
  });
});
