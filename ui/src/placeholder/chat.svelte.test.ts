// M5-T04: the side chat sends the active tab and shows the streamed
// answer, its notes and sources.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { CoreStore } from "../core/store.svelte";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import SideChat from "./SideChat.svelte";

const settle = () => new Promise((r) => setTimeout(r, 0));

test("a message about the tab streams its answer with notes and sources", async () => {
  const seen: Request[] = [];
  let emit: ((e: unknown) => void) | undefined;
  const transport: Transport = {
    label: () => "ui-w1",
    listen: async <T>(_: string, f: (e: T) => void) => {
      emit = f as (e: unknown) => void;
      return () => {};
    },
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value = request.cmd.type === "ai.chat.send" ? { chatId: "c1", messageId: "m1" } : null;
      return { status: "ok", id: request.id, value } as Response;
    },
  };
  const state = $state({
    features: { ai: true },
    profile: { isPrivate: false },
    windows: { w1: { activeTabId: "t1" } },
  });
  render(SideChat, {
    store: { client: new Client(transport), transport, state } as unknown as CoreStore,
    windowId: "w1",
  });
  await fireEvent.click(screen.getByRole("button", { name: "Chat" }));
  await fireEvent.input(screen.getByLabelText("Message"), { target: { value: "Summarise" } });
  await fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await settle();
  expect(seen.map((r) => r.cmd)).toContainEqual({
    type: "ai.chat.send",
    chatId: null,
    text: "Summarise",
    context: { tabIds: ["t1"], selection: null, screenshotTabId: null, files: [] },
  });
  emit?.({
    chatId: "c1",
    messageId: "m1",
    note: { kind: "sensitivePage", url: "https://bank.example/" },
  });
  emit?.({ chatId: "c1", messageId: "m1", delta: "It is " });
  emit?.({ chatId: "c1", messageId: "m1", delta: "short." });
  await settle();
  expect(screen.getByText("It is short.")).toBeTruthy();
  expect(screen.getByText(/https:\/\/bank.example\/ wasn't sent/)).toBeTruthy();
  await fireEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(seen.map((r) => r.cmd)).toContainEqual({ type: "ai.chat.cancel", chatId: "c1" });
  emit?.({
    chatId: "c1",
    messageId: "m1",
    done: true,
    text: "It is short.",
    sources: [{ url: "https://a.example/", title: "A" }],
    notes: [],
    error: null,
  });
  await settle();
  expect(screen.getByText("[1] A")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Send" })).toBeTruthy();
});

test("private windows and AI off show no chat", () => {
  const transport = {
    label: () => "ui-w1",
    listen: async () => () => {},
    invoke: async () => ({}),
  } as unknown as Transport;
  const state = $state({ features: { ai: true }, profile: { isPrivate: true }, windows: {} });
  render(SideChat, {
    store: { client: new Client(transport), transport, state } as unknown as CoreStore,
    windowId: "w1",
  });
  expect(screen.queryByRole("button", { name: "Chat" })).toBeNull();
});
