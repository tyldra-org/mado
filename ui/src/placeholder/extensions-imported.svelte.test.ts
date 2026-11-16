// M3-T15: Settings › Extensions offers extensions an import found in
// another browser, through the store install, until dismissed.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import ExtensionsSection from "./ExtensionsSection.svelte";

const ID = "nngceckbapebfimnlniiiahkandclblb";

test("an imported extension can be added or dismissed", async () => {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "settings",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value = request.cmd.type === "extension.overlaps" ? [] : null;
      return { status: "ok", id: request.id, value } as Response;
    },
  };
  const store = {
    client: new Client(transport),
    state: {
      settings: { values: {}, changed: [] },
      extensions: {
        toolbar: [],
        pinned: [],
        installed: [],
        controlled: {},
        imported: [{ id: ID, name: "Bitwarden", browser: "chrome" }],
      },
    },
  } as unknown as CoreStore;
  render(ExtensionsSection, { store });
  expect(screen.getByText("Bitwarden, from Chrome")).toBeTruthy();
  const sent = () => seen.map((r) => r.cmd).filter((c) => c.type !== "extension.overlaps");
  await fireEvent.click(screen.getByRole("button", { name: "Add Bitwarden to Mado" }));
  await fireEvent.click(screen.getByRole("button", { name: "Don't offer Bitwarden again" }));
  await fireEvent.click(screen.getByRole("button", { name: "Don't offer these again" }));
  expect(sent()).toEqual([
    { type: "extension.install", id: ID, source: "store", path: null },
    { type: "extension.dismissImported", id: ID },
    { type: "extension.dismissImported", id: null },
  ]);
});
