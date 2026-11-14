// M1-T08, Done when: a change in the settings window updates the main
// window live, and settings search shows the rows the core ranks first.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, test } from "vitest";
import { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Patch } from "../generated/Patch";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import type { State } from "../generated/State";
import { t } from "../i18n";
import MainWindow from "./MainWindow.svelte";
import SettingsWindow from "./SettingsWindow.svelte";

const WINDOW = "01a1d7aa-0000-7000-8000-000000000001";

// jsdom has no layout, so no ResizeObserver.
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

/** A tiny stand-in for the core: one document, patches to every webview. */
function fakeCore() {
  const state = {
    rev: 0,
    profile: {
      id: "p",
      name: "Personal",
      color: "blue",
      avatar: "",
      isPrivate: false,
      isGuest: false,
    },
    windows: { [WINDOW]: { id: WINDOW, layout: "vertical", overlay: null } },
    notices: [],
    settings: {
      values: {
        "appearance.theme": "system",
        "blocker.enabled": true,
        "tabs.memorySaver": "balanced",
      },
      changed: [],
    },
    features: {},
  } as unknown as State;
  const listeners: ((p: Patch) => void)[] = [];
  const seen: Request[] = [];
  const transport = (label: string): Transport => ({
    label: () => label,
    listen: async <T>(_event: string, handler: (payload: T) => void) => {
      listeners.push(handler as (p: Patch) => void);
      return () => {};
    },
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const cmd = request.cmd;
      if (cmd.type === "state.get") {
        return {
          status: "ok",
          id: request.id,
          value: { rev: state.rev, state: structuredClone(state) },
        };
      }
      if (cmd.type === "settings.set") {
        state.rev += 1;
        state.settings.values[cmd.key] = cmd.value;
        const patch: Patch = {
          rev: state.rev,
          ops: [{ op: "add", path: `/settings/values/${cmd.key}`, value: cmd.value }],
        };
        for (const l of listeners) l(patch);
        return { status: "ok", id: request.id, value: null };
      }
      if (cmd.type === "settings.search") {
        // The core ranks (mado-settings tests); this answers the test's query.
        return {
          status: "ok",
          id: request.id,
          value: [{ key: "blocker.enabled", section: "privacy", score: 12 }],
        };
      }
      return { status: "ok", id: request.id, value: null };
    },
  });
  return { transport, seen };
}

describe("settings window", () => {
  test("a change in the settings window updates the main window live", async () => {
    const core = fakeCore();
    const main = new CoreStore(core.transport(`ui-${WINDOW}`));
    const settings = new CoreStore(core.transport("settings"));
    await main.start();
    await settings.start();
    const { container } = render(MainWindow, { store: main, windowId: WINDOW });
    render(SettingsWindow, { store: settings });
    await fireEvent.click(screen.getByRole("button", { name: t("settings.section.appearance") }));
    const theme = screen.getByLabelText(t("settings.appearance.theme.label"));
    await fireEvent.change(theme, { target: { value: "dark" } });
    await tick();
    await tick();
    expect(core.seen.some((r) => r.cmd.type === "settings.set" && r.cmd.value === "dark")).toBe(
      true,
    );
    expect(container.querySelector("[data-theme]")?.getAttribute("data-theme")).toBe("dark");
  });

  test("search shows the rows the core returns, in its order", async () => {
    const core = fakeCore();
    const settings = new CoreStore(core.transport("settings"));
    await settings.start();
    render(SettingsWindow, { store: settings });
    const search = screen.getByRole("searchbox", { name: t("settings.search.label") });
    await fireEvent.input(search, { target: { value: "block ads" } });
    await tick();
    await tick();
    expect(
      screen.getAllByRole("checkbox", { name: t("settings.blocker.enabled.label") }),
    ).toHaveLength(1);
    expect(screen.queryByLabelText(t("settings.tabs.memorySaver.label"))).toBeNull();
  });
});
