// M4-T03: the fill menu lists items, sends vault.fill, and offers
// unlocking when the vault is locked.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import FillMenu from "./FillMenu.svelte";

const W = "01a1d7aa-0000-7000-8000-000000000001";
const T = "01a1d7aa-0000-7000-8000-00000000000a";
const P = "01a1d7aa-0000-7000-8000-0000000000aa";

function setup(locked: boolean) {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => `ui-${W}`,
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      return { status: "ok", id: request.id, value: null } as Response;
    },
  };
  const store = {
    client: new Client(transport),
    state: {
      prompts: {
        [P]: {
          type: "fillMenu",
          id: P,
          windowId: W,
          tabId: T,
          formKey: "ab12",
          formKind: "login",
          rect: { x: 10, y: 20, width: 200, height: 30 },
          locked,
          loading: false,
          items: [
            {
              itemRef: "mado:1",
              title: "example.com",
              subtitle: "me@example.com",
              kind: "login",
              source: "mado",
            },
          ],
        },
      },
    },
  } as unknown as CoreStore;
  render(FillMenu, { store, windowId: W });
  return seen;
}

test("choosing an item fills the form", async () => {
  const seen = setup(false);
  await fireEvent.click(screen.getByRole("menuitem", { name: /example\.com/ }));
  expect(seen.map((r) => r.cmd)).toEqual([
    { type: "vault.fill", tabId: T, formKey: "ab12", itemRef: "mado:1" },
  ]);
});

test("a locked vault offers to unlock", async () => {
  const seen = setup(true);
  await fireEvent.click(screen.getByRole("menuitem", { name: "Unlock to fill" }));
  expect(seen.map((r) => r.cmd)).toEqual([{ type: "vault.unlock" }]);
});
