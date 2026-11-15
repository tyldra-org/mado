// M2-T09: the Profiles section asks before deleting and saves link rules.
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { expect, test } from "vitest";
import { Client } from "../core/client";
import type { Transport } from "../core/transport";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";
import ProfilesSection from "./ProfilesSection.svelte";

const HOME = "01a1d7aa-0000-7000-8000-000000000001";
const WORK = "01a1d7aa-0000-7000-8000-000000000002";

test("delete asks first, and a rule is added", async () => {
  const seen: Request[] = [];
  const transport: Transport = {
    label: () => "settings",
    listen: async () => () => {},
    invoke: async (request: Request): Promise<Response> => {
      seen.push(request);
      const value =
        request.cmd.type === "profile.list"
          ? [
              { id: HOME, name: "Home", color: "blue", current: true, running: true },
              { id: WORK, name: "Work", color: "red", current: false, running: false },
            ]
          : request.cmd.type === "profile.linkRules"
            ? []
            : null;
      return { status: "ok", id: request.id, value } as Response;
    },
  };
  render(ProfilesSection, { client: new Client(transport) });
  await new Promise((r) => setTimeout(r, 0));
  await tick();
  const sent = (type: string) => seen.filter((r) => r.cmd.type === type).map((r) => r.cmd);
  await fireEvent.click(screen.getByRole("button", { name: "Delete…" }));
  expect(sent("profile.delete")).toEqual([]);
  await fireEvent.click(screen.getByRole("button", { name: "Delete Work and its data" }));
  expect(sent("profile.delete")).toEqual([{ type: "profile.delete", id: WORK }]);
  await fireEvent.input(screen.getByRole("textbox", { name: "Site" }), {
    target: { value: "*.acme.com" },
  });
  await fireEvent.change(screen.getByRole("combobox", { name: "Profile" }), {
    target: { value: WORK },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Add rule" }));
  expect(sent("profile.linkRules.set")).toEqual([
    { type: "profile.linkRules.set", rules: [{ pattern: "*.acme.com", profileId: WORK }] },
  ]);
});
