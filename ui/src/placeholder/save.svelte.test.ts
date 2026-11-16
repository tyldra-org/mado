// M4-T04: the save prompt sends the edited username, and unlocks first
// when the vault is locked.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import SavePrompt from "./SavePrompt.svelte";

const W = "01a1d7aa-0000-7000-8000-000000000001";
const T = "01a1d7aa-0000-7000-8000-00000000000a";
const P = "01a1d7aa-0000-7000-8000-0000000000aa";

test("Save sends the edited username; a locked vault unlocks first", async () => {
  const seen: Request[] = [];
  let locked = true;
  const transport: Transport = {
    label: () => `ui-${W}`,
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      if (request.cmd.type === "vault.unlock") locked = false;
      if (request.cmd.type === "vault.saveRespond" && locked) {
        return {
          status: "err",
          id: request.id,
          error: { code: "locked", messageKey: "error.vault.locked", args: {}, retryable: false },
        } as Response;
      }
      return { status: "ok", id: request.id, value: null } as Response;
    },
  };
  const store = {
    client: new Client(transport),
    state: {
      prompts: {
        [P]: {
          type: "savePassword",
          id: P,
          windowId: W,
          tabId: T,
          site: "example.com",
          username: "me",
          update: false,
        },
      },
    },
  } as unknown as CoreStore;
  render(SavePrompt, { store, windowId: W });
  expect(screen.getByText("Save password for example.com?")).toBeTruthy();
  await fireEvent.input(screen.getByRole("textbox", { name: "Username" }), {
    target: { value: "me@example.com" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await new Promise((r) => setTimeout(r, 0));
  const save = {
    type: "vault.saveRespond",
    promptId: P,
    action: "save",
    username: "me@example.com",
    password: null,
  };
  expect(seen.map((r) => r.cmd)).toEqual([save, { type: "vault.unlock" }, save]);
});
