// M5-T12: the task list starts, stops, answers and raises limits.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import TasksPanel from "./TasksPanel.svelte";

test("tasks show their status and take answers", async () => {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "ui-w1",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      return { status: "ok", id: request.id, value: { taskId: "t9" } } as Response;
    },
  };
  const task = (id: string, status: string, step: string) => ({
    id,
    title: `Task ${id}`,
    status,
    step,
    groupId: null,
    costMicros: 0,
  });
  const state = $state({
    features: { agents: true },
    ai: { tasks: { a: task("a", "waiting", "Which size?"), b: task("b", "paused", "spendLimit") } },
  });
  render(TasksPanel, { store: { client: new Client(transport), state } as unknown as CoreStore });
  expect(screen.getByText(/Needs you/)).toBeTruthy();
  await fireEvent.input(screen.getByLabelText("Answer"), { target: { value: "Large" } });
  await fireEvent.click(screen.getByRole("button", { name: "Answer" }));
  await fireEvent.click(screen.getByRole("button", { name: "Raise limit for this task" }));
  await fireEvent.input(screen.getByLabelText("Ask the agent to"), {
    target: { value: "find a lamp" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Start" }));
  const cmds = seen.map((r) => r.cmd);
  expect(cmds).toContainEqual({ type: "ai.task.answer", taskId: "a", text: "Large" });
  expect(cmds).toContainEqual({ type: "ai.task.raiseLimit", taskId: "b" });
  expect(cmds.some((c) => c.type === "ai.task.start")).toBe(true);
});
