// M2-T15: menu actions the UI carries out, and the shortcut recorder.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, test } from "vitest";
import type { Client } from "../core/client";
import type { SettingEntry } from "../core/settings-index";
import { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Command } from "../generated/Command";
import FindBar from "./FindBar.svelte";
import SettingRow from "./SettingRow.svelte";

const W = "01a1d7aa-0000-7000-8000-000000000001";
const A = "01a1d7aa-0000-7000-8000-00000000000a";

describe("menu actions (01 §5.4)", () => {
  test("⌘F opens the find bar and ⌘G finds the next match", async () => {
    const seen: Command[] = [];
    let action: ((p: unknown) => void) | undefined;
    const transport: Transport = {
      label: () => `ui-${W}`,
      listen: async (event, handler) => {
        if (event === "mado://action") action = handler as (p: unknown) => void;
        return () => {};
      },
      invoke: async (request) => {
        seen.push(request.cmd);
        const value =
          request.cmd.type === "state.get"
            ? {
                rev: 0,
                state: { windows: { [W]: { id: W, activeTabId: A, findBar: null } }, tabs: {} },
              }
            : null;
        return { status: "ok", id: request.id, value };
      },
    };
    const store = new CoreStore(transport);
    await store.start();
    render(FindBar, { store, windowId: W });
    expect(screen.queryByRole("search")).toBeNull();
    action?.({ action: "find" });
    await tick();
    const input = screen.getByRole("searchbox", { name: "Find in page" });
    await fireEvent.input(input, { target: { value: "rust" } });
    action?.({ action: "findNext" });
    await tick();
    const finds = seen.filter((c) => c.type === "tab.find");
    expect(finds.at(-1)).toEqual({
      type: "tab.find",
      tabId: A,
      query: "rust",
      forward: true,
      caseSensitive: false,
      close: false,
    });
    await fireEvent.keyDown(input, { key: "Escape" });
    expect(seen.filter((c) => c.type === "tab.find").at(-1)).toMatchObject({ close: true });
  });
});

describe("shortcut editor (06 §8)", () => {
  test("records the pressed keys by position, clears and resets", async () => {
    const seen: Command[] = [];
    const client = { call: async (c: Command) => (seen.push(c), null) } as unknown as Client;
    const entry = {
      key: "shortcuts.duplicateTab",
      section: "shortcuts",
      label: "menu.duplicateTab",
      help: "settings.shortcut.help",
      keywords: "settings.shortcut.keywords",
      renderer: "shortcut",
      options: null,
    } as unknown as SettingEntry;
    render(SettingRow, { entry, value: "", client });
    const box = screen.getByLabelText("Duplicate Tab");
    await fireEvent.keyDown(box, { code: "KeyK", key: "k", metaKey: true, shiftKey: true });
    await fireEvent.keyDown(box, { code: "KeyK", key: "k" });
    await fireEvent.keyDown(box, { key: "Backspace", code: "Backspace" });
    await fireEvent.click(screen.getByRole("button", { name: "Reset" }));
    expect(seen).toEqual([
      { type: "settings.set", key: "shortcuts.duplicateTab", value: "Shift+Cmd+K", token: null },
      { type: "settings.set", key: "shortcuts.duplicateTab", value: "", token: null },
      { type: "settings.reset", key: "shortcuts.duplicateTab" },
    ]);
  });
});
