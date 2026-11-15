// M2-T12: the downloads bubble lists downloads with the actions their state
// allows and sends the download commands of spec 12 §4.3.
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
const D1 = "01a1d7aa-0000-7000-8000-0000000000d1";
const D2 = "01a1d7aa-0000-7000-8000-0000000000d2";

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

const download = (id: string, filename: string, extra: Record<string, unknown>) => ({
  id,
  filename,
  url: `https://files.example/${filename}`,
  state: "in_progress",
  received: 0,
  total: null,
  bytesPerSecond: 0,
  danger: null,
  error: null,
  startedAt: 1,
  ...extra,
});

describe("downloads bubble", () => {
  test("offers Keep and Discard for a held file and Pause for a running one", async () => {
    const seen = await shown({
      downloads: {
        [D1]: download(D1, "Setup.dmg", { state: "dangerous", danger: "dangerousType" }),
        [D2]: download(D2, "big.iso", { received: 500, total: 1000, startedAt: 2 }),
      },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Downloads (2)" }));
    expect(screen.getByText(/can harm your Mac.*files\.example/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Keep" }));
    expect(sent(seen, "download.keep")).toEqual([{ type: "download.keep", downloadId: D1 }]);
    await fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(sent(seen, "download.pause")).toEqual([{ type: "download.pause", downloadId: D2 }]);
    await fireEvent.click(screen.getByRole("button", { name: "Clear list" }));
    expect(sent(seen, "download.clearList")).toEqual([{ type: "download.clearList" }]);
  });

  test("an interrupted download can be resumed", async () => {
    const seen = await shown({
      downloads: { [D1]: download(D1, "big.iso", { state: "failed", error: "interrupted" }) },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Downloads (1)" }));
    await fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(sent(seen, "download.resume")).toEqual([{ type: "download.resume", downloadId: D1 }]);
  });
});
