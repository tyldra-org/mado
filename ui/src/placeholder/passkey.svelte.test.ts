// M4-T09: the passkey sheets send vault.passkeyRespond.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import PasskeySheet from "./PasskeySheet.svelte";

const W = "01a1d7aa-0000-7000-8000-000000000001";
const T = "01a1d7aa-0000-7000-8000-00000000000a";
const P = "01a1d7aa-0000-7000-8000-0000000000aa";

function setup(mode: "create" | "get") {
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
          type: "passkey",
          id: P,
          windowId: W,
          tabId: T,
          mode,
          rpId: "example.com",
          userName: "me@example.com",
          accounts: [{ itemId: "i1", userName: "me@example.com", userDisplayName: "Me" }],
          locked: false,
        },
      },
    },
  } as unknown as CoreStore;
  render(PasskeySheet, { store, windowId: W });
  return seen;
}

test("Save in Mado creates the passkey", async () => {
  const seen = setup("create");
  expect(screen.getByText("Create a passkey for example.com?")).toBeTruthy();
  await fireEvent.click(screen.getByRole("button", { name: "Save in Mado" }));
  expect(seen[0]?.cmd).toEqual({
    type: "vault.passkeyRespond",
    promptId: P,
    action: "save",
    itemId: null,
  });
});

test("choosing an account signs in", async () => {
  const seen = setup("get");
  await fireEvent.click(screen.getByRole("button", { name: /me@example\.com/ }));
  expect(seen[0]?.cmd).toEqual({
    type: "vault.passkeyRespond",
    promptId: P,
    action: "use",
    itemId: "i1",
  });
});
