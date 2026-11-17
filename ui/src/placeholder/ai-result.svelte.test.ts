// M5-T05: the result card streams and answers with Replace, Retry with
// another model and Close.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import AiResultCard from "./AiResultCard.svelte";

const settle = () => new Promise((r) => setTimeout(r, 0));

test("a finished answer can replace, retry with another model and close", async () => {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "ui-w1",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value =
        request.cmd.type === "ai.model.list"
          ? [{ connectionId: "c", modelId: "m2", name: "Model Two", show: true }]
          : null;
      return { status: "ok", id: request.id, value } as Response;
    },
  };
  const card = {
    type: "aiResult",
    id: "p1",
    windowId: "w1",
    tabId: "t1",
    commandId: "improveWriting",
    name: "ai.command.improveWriting",
    text: "The cat",
    done: false,
    error: null,
    editable: true,
    model: null,
  };
  const state = $state({ prompts: { p1: card } });
  render(AiResultCard, {
    store: { client: new Client(transport), state } as unknown as CoreStore,
    windowId: "w1",
  });
  expect(screen.getByText("Improve writing")).toBeTruthy();
  expect((screen.getByRole("button", { name: "Replace" }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  state.prompts.p1 = { ...card, text: "The cat sat.", done: true };
  await settle();
  await fireEvent.click(screen.getByRole("button", { name: "Replace" }));
  const select = screen.getByLabelText("Another model");
  await fireEvent.focus(select);
  await settle();
  await fireEvent.change(select, { target: { value: "c:m2" } });
  await fireEvent.click(screen.getByRole("button", { name: "Retry with it" }));
  await fireEvent.click(screen.getByRole("button", { name: "Close" }));
  const cmds = seen.map((r) => r.cmd);
  expect(cmds).toContainEqual({
    type: "ai.command.respond",
    promptId: "p1",
    action: "replace",
    model: null,
  });
  expect(cmds).toContainEqual({
    type: "ai.command.respond",
    promptId: "p1",
    action: "retry",
    model: "c:m2",
  });
  expect(cmds).toContainEqual({
    type: "ai.command.respond",
    promptId: "p1",
    action: "close",
    model: null,
  });
});
