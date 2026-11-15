// How the UI reaches the core: the mado_invoke Tauri command and the
// mado://patch and mado://notice events (mado-docs/spec/01 section 4.1).
// Components never call Tauri directly; tests pass a fake transport.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { MadoError } from "../generated/MadoError";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";

export interface Transport {
  invoke(request: Request): Promise<Response>;
  listen<T>(event: string, handler: (payload: T) => void): Promise<() => void>;
  /** This webview's label: `ui-<WindowId>`, `overlay-<WindowId>` or `settings`. */
  label(): string;
}

/**
 * The transport of `mado://` pages in tabs (spec 01 §4.4): no Tauri, only
 * DOM events the `internal` page script carries to and from the core, as
 * JSON strings. Patches carry only the fields the page reads.
 */
export function pageTransport(doc: Document = document): Transport {
  let next = 0;
  const pending = new Map<number, (r: Response) => void>();
  doc.addEventListener("mado:invoke-reply", (e) => {
    const detail = (e as CustomEvent).detail;
    if (typeof detail !== "string") return;
    const m = JSON.parse(detail) as {
      id: number;
      ok: boolean;
      value?: unknown;
      error?: MadoError;
    };
    const done = pending.get(m.id);
    if (!done) return;
    pending.delete(m.id);
    done(
      m.ok
        ? { status: "ok", id: "", value: m.value }
        : { status: "err", id: "", error: m.error as MadoError },
    );
  });
  return {
    invoke: (request) =>
      new Promise<Response>((resolve) => {
        next += 1;
        const id = next;
        pending.set(id, (r) => resolve({ ...r, id: request.id }));
        doc.dispatchEvent(
          new CustomEvent("mado:invoke", { detail: JSON.stringify({ id, command: request.cmd }) }),
        );
      }),
    listen: async (event, handler) => {
      if (event !== "mado://patch") return () => {};
      const on = (e: Event) => {
        const detail = (e as CustomEvent).detail;
        if (typeof detail === "string") handler(JSON.parse(detail) as never);
      };
      doc.addEventListener("mado:patch", on);
      return () => doc.removeEventListener("mado:patch", on);
    },
    label: () => "internal",
  };
}

export const tauri: Transport = {
  invoke: (request) => invoke<Response>("mado_invoke", { request }),
  listen: (event, handler) => listen(event, (e) => handler(e.payload as never)),
  label: () => getCurrentWebview().label,
};
