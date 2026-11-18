// M5-T14: Settings › AI lists memory files and skills, opens, edits and
// deletes them, files the inbox, undoes a day and pauses memory.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import MemorySection from "./MemorySection.svelte";

const settle = () => new Promise((r) => setTimeout(r, 0));

test("memory files and skills", async () => {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "settings",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const values: Record<string, unknown> = {
        "ai.memory.list": [{ path: "USER.md", bytes: 10 }],
        "ai.memory.read": { path: "USER.md", text: "# Me" },
        "ai.skill.list": [
          { name: "research", description: "Answer from sources", builtin: true },
          { name: "book-a-table", description: "Book a table", builtin: false },
        ],
      };
      return { status: "ok", id: request.id, value: values[request.cmd.type] ?? null } as Response;
    },
  };
  const state = $state({ features: { ai: true, memory: true } });
  render(MemorySection, {
    store: { client: new Client(transport), state } as unknown as CoreStore,
  });
  await settle();
  await settle();
  expect(screen.getByText("Built in")).toBeTruthy();
  await fireEvent.click(screen.getByRole("button", { name: "Open" }));
  await settle();
  await fireEvent.input(screen.getByLabelText("USER.md"), { target: { value: "# Me\n- Tea" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));
  await fireEvent.click(screen.getByRole("button", { name: "File the inbox now" }));
  await fireEvent.input(screen.getByLabelText("Undo a day's filing"), {
    target: { value: "2026-11-18" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  const deletes = screen.getAllByRole("button", { name: "Delete" });
  await fireEvent.click(deletes.at(0) as HTMLElement);
  await fireEvent.click(deletes.at(1) as HTMLElement);
  await fireEvent.click(screen.getByRole("button", { name: "Pause memory" }));
  await settle();
  const cmds = seen.map((r) => r.cmd);
  expect(cmds).toContainEqual({ type: "ai.memory.write", path: "USER.md", text: "# Me\n- Tea" });
  expect(cmds).toContainEqual({ type: "ai.memory.file" });
  expect(cmds).toContainEqual({ type: "ai.memory.undo", day: "2026-11-18" });
  expect(cmds).toContainEqual({ type: "ai.memory.delete", path: "USER.md" });
  expect(cmds).toContainEqual({ type: "ai.skill.delete", name: "book-a-table" });
  expect(cmds).toContainEqual({
    type: "settings.set",
    key: "features.memory",
    value: false,
    token: null,
  });
});
