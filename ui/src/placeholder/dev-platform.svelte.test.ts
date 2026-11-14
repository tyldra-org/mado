// The platform test page calls mado_dev_platform with each row's action and
// shows the answer or the error (M1-T10).
import { fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { describe, expect, test, vi } from "vitest";

const calls: { action: string; arg: string }[] = [];
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (_cmd: string, args: { action: string; arg: string }) => {
    calls.push(args);
    if (args.action === "auth.authenticate") throw "authentication failed";
    return args.action === "keychain.get" ? "secret-value" : null;
  },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));

const { default: DevPlatform } = await import("./DevPlatform.svelte");

describe("platform test page", () => {
  test("each button runs its action and shows the result", async () => {
    render(DevPlatform);
    await fireEvent.click(screen.getByRole("button", { name: "Keychain: read" }));
    await fireEvent.click(screen.getByRole("button", { name: "camera: status" }));
    await fireEvent.click(
      screen.getByRole("button", { name: "Authenticate (Touch ID or password)" }),
    );
    await tick();
    await tick();
    expect(calls).toEqual([
      { action: "keychain.get", arg: "" },
      { action: "permission.status", arg: "camera" },
      { action: "auth.authenticate", arg: "" },
    ]);
    const outputs = screen.getAllByRole("status").map((o) => o.textContent);
    expect(outputs).toContain('"secret-value"');
    expect(outputs).toContain("error: authentication failed");
  });
});
