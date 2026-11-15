// M2-T07: page dialogs answer prompt.answerDialog with the typed text and
// the "no more dialogs" choice.
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

describe("page dialogs", () => {
  test("a prompt dialog sends its text and the suppress choice", async () => {
    const P = "01a1d7aa-0000-7000-8000-0000000000fd";
    const seen = await shown({
      prompts: {
        [P]: {
          type: "dialog",
          id: P,
          tabId: A,
          origin: "https://app.example",
          kind: "prompt",
          message: "Your name?",
          defaultText: "Ada",
          offerSuppress: true,
        },
      },
    });
    expect(screen.getByText("Your name?")).toBeTruthy();
    const input = screen.getByRole("textbox", { name: "Answer" }) as HTMLInputElement;
    expect(input.value).toBe("Ada");
    await fireEvent.input(input, { target: { value: "Grace" } });
    await fireEvent.click(
      screen.getByRole("checkbox", { name: "Don't allow this page to show more dialogs" }),
    );
    await fireEvent.click(screen.getByRole("button", { name: "OK" }));
    expect(sent(seen, "prompt.answerDialog")).toEqual([
      { type: "prompt.answerDialog", id: P, accept: true, text: "Grace", suppress: true },
    ]);
  });
});
