// M5-T13: Settings › AI lists the subscription CLIs, shows the vendor's
// command before running it in Terminal, and connects a ready one.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import AiClis from "./AiClis.svelte";

const settle = () => new Promise((r) => setTimeout(r, 0));
const cli = (provider: string, name: string, extra: object) => ({
  provider,
  name,
  version: null,
  needsNode: true,
  nodeOk: true,
  install: `install ${provider}`,
  login: `login ${provider}`,
  loginHint: null,
  ready: false,
  ...extra,
});

test("CLIs show how to install, log in and connect", async () => {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "settings",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value =
        request.cmd.type === "ai.cli.list"
          ? [
              cli("claudeCode", "Claude Code", {
                version: "2.1.0",
                ready: true,
                loginHint: "Type /login.",
              }),
              cli("codex", "Codex", {}),
              cli("geminiCli", "Gemini CLI", { version: "0.6", nodeOk: false }),
            ]
          : null;
      return { status: "ok", id: request.id, value } as Response;
    },
  };
  const state = $state({ features: { ai: true }, ai: { connections: {} } });
  render(AiClis, { store: { client: new Client(transport), state } as unknown as CoreStore });
  await settle();
  await settle();
  expect(screen.getByText("Runs in Terminal: install codex")).toBeTruthy();
  expect(screen.getByText("Type /login.")).toBeTruthy();
  expect(screen.getByText(/needs Node.js 18/)).toBeTruthy();
  await fireEvent.click(screen.getByRole("button", { name: "Install in Terminal" }));
  await fireEvent.click(
    screen.getAllByRole("button", { name: "Log in in Terminal" }).at(0) as HTMLElement,
  );
  await fireEvent.click(screen.getByRole("button", { name: "Use Claude Code with Mado" }));
  const cmds = seen.map((r) => r.cmd);
  expect(cmds).toContainEqual({ type: "ai.cli.run", provider: "codex", what: "install" });
  expect(cmds).toContainEqual({ type: "ai.cli.run", provider: "claudeCode", what: "login" });
  expect(cmds).toContainEqual({
    type: "ai.connection.add",
    kind: "subscriptionCli",
    provider: "claudeCode",
    label: null,
    baseUrl: null,
    secret: null,
  });
});
