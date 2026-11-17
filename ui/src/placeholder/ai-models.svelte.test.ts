// M5-T03: role rows and the model picker send their commands.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import AiModels from "./AiModels.svelte";

const settle = () => new Promise((r) => setTimeout(r, 0));

test("roles pick a model, effort and fallback; models show and star", async () => {
  const seen: Request[] = [];
  const models = [
    {
      connectionId: "c",
      modelId: "m1",
      name: "Model One",
      show: true,
      starred: false,
      info: { pricePerMTok: { input: 3, output: 15 } },
    },
    { connectionId: "c", modelId: "m2", name: "Model Two", show: true, starred: false, info: null },
  ];
  const transport: Transport = {
    label: () => "settings",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value = request.cmd.type === "ai.model.list" ? models : null;
      return { status: "ok", id: request.id, value } as Response;
    },
  };
  const role = { model: "auto", effort: "auto", fallback: null, resolved: "c:m1" };
  const state = $state({
    features: { ai: true },
    ai: { connections: {}, tasks: {}, roles: { chat: role, tasks: role } },
  });
  render(AiModels, { store: { client: new Client(transport), state } as unknown as CoreStore });
  await settle();
  await settle();
  expect(screen.getAllByText("Automatic (Model One)").length).toBeGreaterThan(0);
  expect(screen.getByText(/\$3 in, \$15 out/)).toBeTruthy();
  expect(screen.getByText(/Unknown price/)).toBeTruthy();
  const [chatModel] = screen.getAllByLabelText("Model");
  await fireEvent.change(chatModel as HTMLElement, { target: { value: "c:m2" } });
  const [chatEffort] = screen.getAllByLabelText("Thinking");
  await fireEvent.change(chatEffort as HTMLElement, { target: { value: "high" } });
  const [chatFallback] = screen.getAllByLabelText("If it fails");
  await fireEvent.change(chatFallback as HTMLElement, { target: { value: "c:m1" } });
  const [star] = screen.getAllByLabelText("Star");
  await fireEvent.click(star as HTMLElement);
  await settle();
  const cmds = seen.map((r) => r.cmd);
  expect(cmds).toContainEqual({
    type: "ai.role.set",
    role: "chat",
    model: "c:m2",
    effort: "auto",
    fallback: null,
  });
  expect(cmds).toContainEqual({
    type: "ai.role.set",
    role: "chat",
    model: "auto",
    effort: "high",
    fallback: null,
  });
  expect(cmds).toContainEqual({
    type: "ai.role.set",
    role: "chat",
    model: "auto",
    effort: "auto",
    fallback: "c:m1",
  });
  expect(cmds).toContainEqual({
    type: "ai.model.set",
    connectionId: "c",
    modelId: "m1",
    show: null,
    starred: true,
  });
});
