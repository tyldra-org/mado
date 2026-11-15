// M2-T14: mado:// pages reach the core through DOM events the `internal`
// page script carries, and mirror only the state they read.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, describe, expect, test } from "vitest";
import { CoreStore } from "../core/store.svelte";
import { pageTransport } from "../core/transport";
import Internal from "./Internal.svelte";

const settle = async () => {
  for (let i = 0; i < 3; i++) {
    await new Promise((r) => setTimeout(r, 0));
    await tick();
  }
};

type Cmd = { type: string } & Record<string, unknown>;

/** Plays the internal page script and the core: answers mado:invoke. */
function fakeCore(answer: (cmd: Cmd) => unknown) {
  const seen: Cmd[] = [];
  const on = (e: Event) => {
    const { id, command } = JSON.parse((e as CustomEvent).detail) as { id: number; command: Cmd };
    seen.push(command);
    let reply: unknown;
    try {
      reply = { type: "invokeReply", id, ok: true, value: answer(command) };
    } catch (err) {
      reply = { type: "invokeReply", id, ok: false, error: err };
    }
    queueMicrotask(() =>
      document.dispatchEvent(
        new CustomEvent("mado:invoke-reply", { detail: JSON.stringify(reply) }),
      ),
    );
  };
  document.addEventListener("mado:invoke", on);
  const stop = () => document.removeEventListener("mado:invoke", on);
  return { seen, stop };
}

const ids = {
  tabId: "01a1d7aa-0000-7000-8000-00000000000a",
  windowId: "01a1d7aa-0000-7000-8000-000000000001",
};
let stop = () => {};
afterEach(() => stop());

async function show(page: string, answer: (cmd: Cmd) => unknown) {
  const core = fakeCore(answer);
  stop = core.stop;
  const store = new CoreStore(pageTransport());
  render(Internal, { store, page });
  await store.start();
  await settle();
  return { seen: core.seen, store };
}

describe("internal pages (01 §4.4)", () => {
  test("history lists visits, opens one from this tab and deletes one", async () => {
    const { seen } = await show("history", (cmd) => {
      if (cmd.type === "state.get") return { rev: 1, state: {}, ...ids };
      if (cmd.type === "history.query")
        return {
          days: [
            {
              day: "2026-11-15",
              visits: [{ visitId: 7, url: "https://a.example/", title: "A", visitedAt: 1 }],
            },
          ],
          cursor: null,
        };
      return null;
    });
    expect(document.title).toBe("History");
    await fireEvent.click(screen.getByRole("button", { name: "A" }));
    await settle();
    expect(seen.find((c) => c.type === "nav.open")).toMatchObject({
      input: "https://a.example/",
      ...ids,
    });
    await fireEvent.click(screen.getByRole("button", { name: "Remove A from history" }));
    await settle();
    expect(seen.find((c) => c.type === "history.delete")).toMatchObject({ visitIds: [7] });
  });

  test("downloads mirror only their field and follow filtered patches", async () => {
    const d = (state: string) => ({
      id: "d1",
      filename: "file.zip",
      url: "https://a.example/f.zip",
      state,
      received: 1,
      total: 2,
      bytesPerSecond: 0,
      danger: null,
      error: null,
      startedAt: 1,
    });
    const { seen } = await show("downloads", (cmd) =>
      cmd.type === "state.get"
        ? { rev: 3, state: { downloads: { d1: d("in_progress") } }, ...ids }
        : null,
    );
    expect(screen.getByText("Downloading")).toBeTruthy();
    // Revision 9 after 3: an internal page skips revisions it does not read.
    document.dispatchEvent(
      new CustomEvent("mado:patch", {
        detail: JSON.stringify({
          type: "patch",
          rev: 9,
          ops: [{ op: "replace", path: "/downloads/d1/state", value: "complete" }],
        }),
      }),
    );
    await settle();
    expect(screen.getByText("Done")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Show in Finder" }));
    expect(seen.find((c) => c.type === "download.reveal")).toEqual({
      type: "download.reveal",
      downloadId: "d1",
    });
  });

  test("an unknown page says so", async () => {
    await show("nowhere", () => ({ rev: 1, state: {}, ...ids }));
    expect(screen.getByRole("heading", { name: "Mado has no such page." })).toBeTruthy();
  });
});
