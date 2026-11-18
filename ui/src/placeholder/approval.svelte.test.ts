// M5-T11: the approval card shows the total and offers "for this task"
// only where allowed.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import ApprovalCard from "./ApprovalCard.svelte";

test("a payment card shows its total and has no allow-for-task", async () => {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "ui-w1",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      return { status: "ok", id: request.id, value: null } as Response;
    },
  };
  const card = (id: string, action: string, amount: string | null, allowForTask: boolean) => ({
    type: "aiApproval",
    id,
    windowId: "w1",
    tabId: null,
    taskId: "t",
    action,
    detail: `Click "${action}"`,
    site: "shop.example",
    amount,
    allowForTask,
  });
  const state = $state({ prompts: { p1: card("p1", "pay", "$12.00", false) } });
  render(ApprovalCard, {
    store: { client: new Client(transport), state } as unknown as CoreStore,
    windowId: "w1",
  });
  expect(screen.getByText("Total: $12.00")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Allow for this task" })).toBeNull();
  await fireEvent.click(screen.getByRole("button", { name: "Deny" }));
  expect(seen.map((r) => r.cmd)).toContainEqual({
    type: "ai.approval.respond",
    promptId: "p1",
    answer: "deny",
  });
});
