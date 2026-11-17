// M5-T01: Settings › AI lists connections without their keys and sends
// every connection command.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import AiConnections from "./AiConnections.svelte";

const settle = () => new Promise((r) => setTimeout(r, 0));

function setup(on = true) {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "settings",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value =
        request.cmd.type === "ai.connection.detectLocal"
          ? [{ provider: "ollama", baseUrl: "http://localhost:11434/v1", models: 3 }]
          : request.cmd.type === "ai.connection.add"
            ? { id: "new" }
            : null;
      return { status: "ok", id: request.id, value } as Response;
    },
  };
  const conn = (id: string, priority: number, extra: object) => ({
    id,
    kind: "apiKey",
    provider: "anthropic",
    label: id,
    priority,
    status: "ready",
    error: null,
    secretHint: "abcd",
    baseUrl: null,
    models: 4,
    ...extra,
  });
  const state = $state({
    features: { ai: on },
    ai: {
      enabled: on,
      tasks: {},
      connections: {
        a: conn("a", 0, {}),
        b: conn("b", 1, { status: "error", error: "unauthorized", provider: "openai" }),
      },
    },
  });
  const store = { client: new Client(transport), state } as unknown as CoreStore;
  render(AiConnections, { store });
  return seen;
}

test("connections show their status and only the key's last characters", async () => {
  setup();
  await settle();
  expect(screen.getAllByText(/••••abcd/, { selector: "li" }).length).toBe(2);
  expect(screen.getByText(/The key was refused/)).toBeTruthy();
  expect(screen.getAllByText(/4 models/).length).toBe(1);
});

test("adding, signing in, local servers, testing, ordering and removing", async () => {
  const seen = setup();
  await settle();
  await fireEvent.input(screen.getByLabelText("API key"), { target: { value: "sk-1" } });
  await fireEvent.click(screen.getByRole("button", { name: "Add" }));
  await fireEvent.click(screen.getByRole("button", { name: "Sign in with OpenRouter" }));
  await fireEvent.click(screen.getByRole("button", { name: "Use Ollama on this computer" }));
  await fireEvent.click(screen.getAllByRole("button", { name: "Test" }).at(0) as HTMLElement);
  await fireEvent.click(screen.getByRole("button", { name: "Move up" }));
  await fireEvent.input(screen.getAllByLabelText("New key").at(1) as HTMLElement, {
    target: { value: "sk-2" },
  });
  await fireEvent.click(screen.getAllByRole("button", { name: "Save key" }).at(1) as HTMLElement);
  await fireEvent.click(screen.getAllByRole("button", { name: "Remove" }).at(0) as HTMLElement);
  await settle();
  const cmds = seen.map((r) => r.cmd);
  expect(cmds).toContainEqual({
    type: "ai.connection.add",
    kind: "apiKey",
    provider: "anthropic",
    label: null,
    baseUrl: null,
    secret: "sk-1",
  });
  expect(cmds).toContainEqual({
    type: "ai.connection.add",
    kind: "oauth",
    provider: "openrouter",
    label: null,
    baseUrl: null,
    secret: null,
  });
  expect(cmds).toContainEqual({
    type: "ai.connection.add",
    kind: "local",
    provider: "ollama",
    label: null,
    baseUrl: "http://localhost:11434/v1",
    secret: null,
  });
  expect(cmds).toContainEqual({ type: "ai.connection.test", id: "a" });
  expect(cmds).toContainEqual({
    type: "ai.connection.update",
    id: "b",
    label: null,
    priority: -1,
    secret: null,
  });
  expect(cmds).toContainEqual({
    type: "ai.connection.update",
    id: "b",
    label: null,
    priority: null,
    secret: "sk-2",
  });
  expect(cmds).toContainEqual({ type: "ai.connection.remove", id: "a" });
});

test("with AI off the section says how to turn it on", () => {
  const seen = setup(false);
  expect(screen.getByText(/Turn on AI in Settings/)).toBeTruthy();
  expect(seen).toEqual([]);
});
