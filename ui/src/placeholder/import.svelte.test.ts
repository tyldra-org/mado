// M2-T13: the import sheet lists what the core detected, sends the chosen
// types, and explains Full Disk Access, retrying when Mado is back in front.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, test } from "vitest";
import type { Client } from "../core/client";
import { CoreError } from "../core/client";
import type { Command } from "../generated/Command";
import ImportSheet from "./ImportSheet.svelte";

const settle = async () => {
  await new Promise((r) => setTimeout(r, 0));
  await tick();
};

function client(fail: boolean) {
  const seen: Command[] = [];
  let refuse = fail;
  const c = {
    call: async (cmd: Command) => {
      seen.push(cmd);
      if (cmd.type === "import.detect")
        return [
          { browser: "chrome", profile: "Default", name: "Personal" },
          { browser: "safari", profile: "", name: "Safari" },
        ];
      if (cmd.type === "import.run") {
        if (refuse) {
          refuse = false;
          throw new CoreError({
            code: "permissionDenied",
            messageKey: "error.import.fullDiskAccess",
            args: {},
            retryable: false,
          });
        }
        return { bookmarks: 3, visits: 2, engines: 1, tabs: 4, extensions: 0 };
      }
      return null;
    },
  } as unknown as Client;
  return { c, seen };
}

describe("import sheet (12 §5)", () => {
  test("imports the chosen types from the chosen profile", async () => {
    const { c, seen } = client(false);
    render(ImportSheet, { client: c });
    await settle();
    await fireEvent.click(screen.getByRole("checkbox", { name: "Open tabs" }));
    await fireEvent.click(
      screen.getByRole("checkbox", { name: "All history, not only the last 90 days" }),
    );
    await fireEvent.click(screen.getByRole("button", { name: "Import" }));
    await settle();
    expect(seen.filter((s) => s.type === "import.run")).toEqual([
      {
        type: "import.run",
        browser: "chrome",
        profile: "Default",
        types: ["bookmarks", "history", "searchEngines", "tabs"],
        allHistory: true,
      },
    ]);
    expect(screen.getByRole("status").textContent).toContain("3 bookmarks, 2 visits and 4 tabs");
  });

  test("Full Disk Access is explained and the import retries on return", async () => {
    const { c, seen } = client(true);
    render(ImportSheet, { client: c });
    await settle();
    await fireEvent.change(screen.getByRole("combobox", { name: "From" }), {
      target: { value: "1" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Import" }));
    await settle();
    expect(screen.getByRole("alert").textContent).toContain("Full Disk Access");
    await fireEvent.click(screen.getByRole("button", { name: "Open System Settings" }));
    expect(seen).toContainEqual({ type: "app.openSystemSettings", pane: "fullDiskAccess" });
    window.dispatchEvent(new Event("focus"));
    await settle();
    expect(seen.filter((s) => s.type === "import.run")).toHaveLength(2);
    expect(screen.getByRole("status")).toBeTruthy();
  });
});
