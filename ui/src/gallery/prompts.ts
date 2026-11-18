// Prompt fixtures by kind, for the gallery's ?prompt= parameter.
import { TABS, W } from "./fixtures";

export function prompts(kind: string): Record<string, unknown> {
  const id = "01a1d7aa-0000-7000-8000-000000000999";
  const base = { id, windowId: W, tabId: TABS[2] ?? "" };
  const all: Record<string, Record<string, unknown>> = {
    permission: {
      ...base,
      type: "permission",
      origin: "https://journal.example",
      permissions: ["camera", "microphone"],
      remember: true,
    },
    approval: {
      ...base,
      type: "aiApproval",
      taskId: "t",
      action: "pay",
      detail: 'Click "Place order"',
      site: "shop.example",
      amount: "$123.45",
      allowForTask: false,
    },
    save: {
      ...base,
      type: "savePassword",
      site: "journal.example",
      username: "ada@example.com",
      update: false,
    },
  };
  return all[kind] ? { [id]: all[kind] } : {};
}
