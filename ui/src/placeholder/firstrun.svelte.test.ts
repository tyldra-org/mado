// M2-T17: the welcome sheet's steps, in order.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, test } from "vitest";
import { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Command } from "../generated/Command";
import FirstRun from "./FirstRun.svelte";

const settle = async () => {
  for (let i = 0; i < 3; i++) {
    await new Promise((r) => setTimeout(r, 0));
    await tick();
  }
};

describe("first run", () => {
  test("import, then a search engine with suggestions, then the default browser", async () => {
    const seen: Command[] = [];
    const transport: Transport = {
      label: () => "ui-w",
      listen: async () => () => {},
      invoke: async (request) => {
        seen.push(request.cmd);
        const values: Record<string, unknown> = {
          "state.get": { rev: 0, state: { firstRun: true } },
          "import.detect": [],
          "nav.searchEngines": [
            { id: "duckduckgo", name: "DuckDuckGo", active: true },
            { id: "off", name: "Off", active: false },
          ],
          "app.defaultBrowser": { isDefault: false },
        };
        return { status: "ok", id: request.id, value: values[request.cmd.type] ?? null };
      },
    };
    const store = new CoreStore(transport);
    await store.start();
    render(FirstRun, { store });
    await settle();
    expect(screen.getByText("Mado found no other browser on this Mac.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await settle();
    const next = screen.getByRole("button", { name: "Continue" }) as HTMLButtonElement;
    expect(next.disabled).toBe(true);
    expect(screen.queryByLabelText("Off")).toBeNull();
    await fireEvent.click(screen.getByLabelText("DuckDuckGo"));
    await fireEvent.click(screen.getByLabelText("Show search suggestions as you type"));
    await fireEvent.click(next);
    await settle();
    await fireEvent.click(screen.getByRole("button", { name: "Make Mado the default browser" }));
    await settle();
    const types = seen.map((c) => c.type);
    expect(seen).toContainEqual({
      type: "settings.set",
      key: "search.defaultEngine",
      value: "duckduckgo",
      token: null,
    });
    expect(seen).toContainEqual({
      type: "settings.set",
      key: "search.suggestions",
      value: "on",
      token: null,
    });
    expect(types.slice(-2)).toEqual(["app.setDefaultBrowser", "app.finishFirstRun"]);
  });
});
