// M5-T06: the pairing card grants the chosen scopes; Settings › Developers
// lists paired apps and revokes them.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import ControlClients from "./ControlClients.svelte";
import PairingCard from "./PairingCard.svelte";

const settle = () => new Promise((r) => setTimeout(r, 0));

function transport(seen: Request[]): Transport {
  return {
    label: () => "settings",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value =
        request.cmd.type === "control.clients"
          ? [{ id: "c1", name: "mado", scopes: ["read", "full"], revoked: false }]
          : null;
      return { status: "ok", id: request.id, value } as Response;
    },
  };
}

test("the pairing card offers full control to the CLI and sends the chosen scopes", async () => {
  const seen: Request[] = [];
  const state = $state({
    prompts: {
      p1: {
        type: "controlPairing",
        id: "p1",
        clientName: "mado",
        clientVersion: "0.5",
        offerFull: true,
      },
    },
  });
  render(PairingCard, {
    store: { client: new Client(transport(seen)), state } as unknown as CoreStore,
  });
  expect(screen.getByText("Allow mado to control Mado?")).toBeTruthy();
  await fireEvent.click(screen.getByLabelText(/Full control/));
  await fireEvent.click(screen.getByRole("button", { name: "Allow" }));
  expect(seen.map((r) => r.cmd)).toContainEqual({
    type: "control.pairRespond",
    promptId: "p1",
    allow: true,
    scopes: ["read", "full"],
  });
});

test("a card without full control offers none, and Deny sends no scopes", async () => {
  const seen: Request[] = [];
  const state = $state({
    prompts: {
      p2: {
        type: "controlPairing",
        id: "p2",
        clientName: "tool",
        clientVersion: "1",
        offerFull: false,
      },
    },
  });
  render(PairingCard, {
    store: { client: new Client(transport(seen)), state } as unknown as CoreStore,
  });
  expect(screen.queryByLabelText(/Full control/)).toBeNull();
  await fireEvent.click(screen.getByRole("button", { name: "Deny" }));
  expect(seen.map((r) => r.cmd)).toContainEqual({
    type: "control.pairRespond",
    promptId: "p2",
    allow: false,
    scopes: [],
  });
});

test("paired apps can be revoked", async () => {
  const seen: Request[] = [];
  const state = $state({ settings: { values: {} } });
  render(ControlClients, {
    store: { client: new Client(transport(seen)), state } as unknown as CoreStore,
  });
  await settle();
  await fireEvent.click(screen.getByRole("button", { name: "Revoke" }));
  expect(seen.map((r) => r.cmd)).toContainEqual({ type: "control.revoke", clientId: "c1" });
});
