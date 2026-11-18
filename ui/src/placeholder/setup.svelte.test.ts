// M5-T16: Settings › Developers sets Mado up in other agents and removes
// it, and copies the MCP config.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import AgentSetup from "./AgentSetup.svelte";

const settle = () => new Promise((r) => setTimeout(r, 0));

test("set up, remove and copy config", async () => {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "settings",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value =
        request.cmd.type === "control.setupStatus"
          ? {
              targets: [
                { target: "claude", setUp: true, available: true },
                { target: "codex", setUp: false, available: true },
                { target: "cursor", setUp: false, available: false },
              ],
              config: { mcpServers: { mado: { command: "/m", args: ["mcp"] } } },
            }
          : { message: "Added Mado's MCP server." };
      return { status: "ok", id: request.id, value } as Response;
    },
  };
  const writeText = vi.fn(async () => {});
  Object.assign(navigator, { clipboard: { writeText } });
  render(AgentSetup, {
    store: { client: new Client(transport), state: {} } as unknown as CoreStore,
  });
  await settle();
  await settle();
  expect(screen.getByText("Not installed")).toBeTruthy();
  await fireEvent.click(screen.getByRole("button", { name: "Remove" }));
  await fireEvent.click(screen.getByRole("button", { name: "Set up" }));
  await settle();
  expect(screen.getByRole("status").textContent).toBe("Added Mado's MCP server.");
  await fireEvent.click(screen.getByRole("button", { name: "Copy config for other MCP clients" }));
  expect(writeText).toHaveBeenCalledWith(expect.stringContaining('"command": "/m"'));
  const cmds = seen.map((r) => r.cmd);
  expect(cmds).toContainEqual({ type: "control.setup", target: "claude", remove: true });
  expect(cmds).toContainEqual({ type: "control.setup", target: "codex", remove: false });
});
