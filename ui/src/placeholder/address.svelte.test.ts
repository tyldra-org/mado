// M2-T06: the address bar suggests as it is typed, opens on Enter, asks for an
// engine the first time, and the Safari prompt answers nav.openExternally.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, test } from "vitest";
import { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import type { State } from "../generated/State";
import AddressBar from "./AddressBar.svelte";
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

// The field itself: in the window it is the command bar's, in the overlay.
async function field() {
  const { store, seen } = setup();
  await store.start();
  render(AddressBar, { store, windowId: W });
  await tick();
  return seen;
}

const sent = (seen: Request[], type: string) =>
  seen.filter((r) => r.cmd.type === type).map((r) => r.cmd);

describe("address bar", () => {
  test("shows the display URL, suggests while typing and completes inline", async () => {
    const seen = await field();
    const input = screen.getByRole("textbox", { name: "Address and search" }) as HTMLInputElement;
    expect(input.value).toBe("alpha.example");
    expect(screen.getByText("Secure")).toBeTruthy();
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "git" } });
    await new Promise((r) => setTimeout(r, 0));
    await tick();
    expect(sent(seen, "nav.suggest").at(-1)).toMatchObject({ input: "git", windowId: W });
    expect(screen.getByRole("option").textContent).toContain("GitHub");
    expect(screen.getByText("hub.com")).toBeTruthy();
  });

  test("Enter opens what was typed; a search first asks for an engine", async () => {
    const seen = await field();
    const input = screen.getByRole("textbox", { name: "Address and search" });
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "rust book" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    await new Promise((r) => setTimeout(r, 0));
    await tick();
    expect(sent(seen, "nav.open")[0]).toMatchObject({
      input: "rust book",
      disposition: "currentTab",
      source: "typed",
    });
    await fireEvent.click(await screen.findByRole("button", { name: "DuckDuckGo" }));
    await new Promise((r) => setTimeout(r, 0));
    expect(sent(seen, "settings.set")).toEqual([
      { type: "settings.set", key: "search.defaultEngine", value: "duckduckgo", token: null },
    ]);
    expect(sent(seen, "nav.open")).toHaveLength(2);
  });

  test("in the window, the address opens the command bar over the page", async () => {
    const seen = await shown();
    const input = screen.getByRole("textbox", { name: "Address and search" }) as HTMLInputElement;
    expect(input.value).toBe("alpha.example");
    expect(input.readOnly).toBe(true);
    const main = document.querySelector("main") as HTMLElement;
    main.getBoundingClientRect = () =>
      ({ x: 240, y: 48, width: 1000, height: 700, top: 48, left: 240 }) as DOMRect;
    await fireEvent.mouseDown(input);
    expect(sent(seen, "window.overlayOpen")).toMatchObject([
      {
        type: "window.overlayOpen",
        windowId: W,
        kind: "commandBar",
        id: "location",
        rect: { x: 420, y: 60, width: 640, height: 440 },
      },
    ]);
  });

  test("the Safari prompt answers nav.openExternally", async () => {
    const P = "01a1d7aa-0000-7000-8000-0000000000ff";
    const seen = await shown({
      prompts: {
        [P]: { type: "openInSafari", id: P, tabId: A, host: "tv.example", reason: "drm" },
      },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Always for this site" }));
    expect(sent(seen, "nav.openExternally")).toEqual([
      { type: "nav.openExternally", tabId: A, answer: "always" },
    ]);
  });
});
