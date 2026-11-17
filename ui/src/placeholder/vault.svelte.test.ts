// M4-T12: every vault command is reachable from Mado's own windows, and
// the Passwords section turns Mado Passwords on and shows the recovery
// secret once.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import PasswordsSection from "./PasswordsSection.svelte";

test("every vault command is sent by some placeholder", () => {
  const generated = readFileSync(join(__dirname, "../generated/Command.ts"), "utf8");
  const names = [
    ...new Set([...generated.matchAll(/"type": "(vault\.[a-zA-Z]+)"/g)].map((m) => m[1])),
  ];
  expect(names.length).toBeGreaterThan(20);
  const sources = readdirSync(__dirname)
    .filter((f) => f.endsWith(".svelte"))
    .map((f) => readFileSync(join(__dirname, f), "utf8"))
    .join("\n");
  const missing = names.filter((n) => !sources.includes(`"${n}"`));
  expect(missing).toEqual([]);
});

test("turning Mado Passwords on shows the recovery secret", async () => {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "settings",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value =
        request.cmd.type === "vault.setup"
          ? { recoverySecret: "ABCDE-FGHIJ-KLMNO-PQRST-UVWXY-Z" }
          : request.cmd.type === "vault.status"
            ? { status: "none", pendingImport: null }
            : null;
      return { status: "ok", id: request.id, value } as Response;
    },
  };
  const state = $state({ vault: { status: "none" }, windows: {} });
  const store = { client: new Client(transport), state } as unknown as CoreStore;
  render(PasswordsSection, { store });
  await fireEvent.click(screen.getByRole("button", { name: "Turn on Mado Passwords" }));
  state.vault.status = "unlocked";
  await new Promise((r) => setTimeout(r, 0));
  expect(screen.getByText("ABCDE-FGHIJ-KLMNO-PQRST-UVWXY-Z")).toBeTruthy();
  expect(seen.some((r) => r.cmd.type === "vault.query")).toBe(true);
});
