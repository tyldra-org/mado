// M2-T05: the tab strip and sidebar placeholder send the commands of spec
// 04 §2, and the unsaved-text prompt answers with session.confirmClose.
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
const B = "01a1d7aa-0000-7000-8000-00000000000b";
const C = "01a1d7aa-0000-7000-8000-00000000000c";
const G = "01a1d7aa-0000-7000-8000-0000000000aa";

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

const tab = (id: string, title: string, extra: Record<string, unknown> = {}) => ({
  id,
  windowId: W,
  groupId: null,
  pinned: false,
  title,
  customTitle: null,
  url: `https://${title.toLowerCase()}.example/`,
  favicon: null,
  life: "live",
  audio: { playing: false, muted: false },
  ...extra,
});

function setup(extra: Partial<Record<string, unknown>> = {}) {
  const state = {
    rev: 0,
    profile: { id: "p", name: "Personal", color: "", avatar: "", isPrivate: false, isGuest: false },
    windows: {
      [W]: { id: W, layout: "vertical", tabOrder: [A, B, C], activeTabId: A, overlay: null },
    },
    tabs: {
      [A]: tab(A, "Alpha", { pinned: true }),
      [B]: tab(B, "Beta", { groupId: G }),
      [C]: tab(C, "Gamma", { groupId: G, audio: { playing: true, muted: false } }),
    },
    groups: {
      [G]: { id: G, windowId: W, title: "Work", color: "blue", collapsed: false, saved: true },
    },
    prompts: {},
    notices: [],
    settings: { values: {}, changed: [] },
    features: {},
    ...extra,
  } as unknown as State;
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => `ui-${W}`,
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      if (request.cmd.type === "state.get") {
        return { status: "ok", id: request.id, value: { rev: 0, state: structuredClone(state) } };
      }
      return { status: "ok", id: request.id, value: null };
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

describe("tab strip", () => {
  test("lists the window's tabs in order, the active one selected", async () => {
    await shown();
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.textContent?.replace(/\s+/g, " ").trim())).toEqual([
      "Alpha",
      "Beta",
      "Gamma Playing audio",
    ]);
    expect(tabs[0]?.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("button", { name: "Work" }).getAttribute("aria-expanded")).toBe("true");
  });

  test("a click activates, Delete closes, ⌘-click selects several for a group", async () => {
    const seen = await shown();
    const [, beta, gamma] = screen.getAllByRole("tab");
    if (!beta || !gamma) throw new Error("tabs");
    await fireEvent.click(beta);
    expect(sent(seen, "tab.activate")).toEqual([{ type: "tab.activate", tabId: B }]);
    await fireEvent.click(gamma, { metaKey: true });
    await fireEvent.click(screen.getByRole("button", { name: "Group tabs" }));
    expect(sent(seen, "group.create")).toEqual([
      { type: "group.create", tabIds: [B, C], title: null, color: null },
    ]);
    await fireEvent.keyDown(gamma, { key: "Delete" });
    expect(sent(seen, "tab.close")).toEqual([{ type: "tab.close", tabIds: [B, C] }]);
  });

  test("new tab, reopen closed, mute and pin send their commands", async () => {
    const seen = await shown();
    await fireEvent.click(screen.getByRole("button", { name: "New tab" }));
    await fireEvent.click(screen.getByRole("button", { name: "Reopen closed tab" }));
    await fireEvent.click(screen.getByRole("button", { name: "Mute" }));
    await fireEvent.click(screen.getByRole("button", { name: "Unpin" }));
    expect(seen.map((r) => r.cmd.type).filter((t) => t !== "state.get")).toEqual([
      "tab.new",
      "session.reopenClosed",
      "tab.mute",
      "tab.unpin",
    ]);
  });

  test("a collapsed group shows only its active tab", async () => {
    await shown({
      groups: {
        [G]: { id: G, windowId: W, title: "Work", color: "blue", collapsed: true, saved: true },
      },
    });
    expect(
      screen.getAllByRole("tab").map((t) => t.textContent?.replace(/\s+/g, " ").trim()),
    ).toEqual(["Alpha"]);
  });

  test("the unsaved-text prompt lists the tabs and answers", async () => {
    const P = "01a1d7aa-0000-7000-8000-0000000000ff";
    const seen = await shown({
      prompts: {
        [P]: {
          type: "unsavedText",
          id: P,
          windowId: null,
          tabs: [{ tabId: B, title: "Beta", host: "beta.example" }],
        },
      },
    });
    expect(screen.getByRole("alertdialog").textContent).toContain("beta.example");
    await fireEvent.click(screen.getByRole("button", { name: "Quit anyway" }));
    expect(sent(seen, "session.confirmClose")).toEqual([
      { type: "session.confirmClose", promptId: P, proceed: true },
    ]);
  });
});
