// How the UI reaches the core: the mado_invoke Tauri command and the
// mado://patch and mado://notice events (mado-docs/spec/01 section 4.1).
// Components never call Tauri directly; tests pass a fake transport.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Request } from "../generated/Request";
import type { Response } from "../generated/Response";

export interface Transport {
  invoke(request: Request): Promise<Response>;
  listen<T>(event: string, handler: (payload: T) => void): Promise<() => void>;
  /** This webview's label: `ui-<WindowId>`, `overlay-<WindowId>` or `settings`. */
  label(): string;
}

export const tauri: Transport = {
  invoke: (request) => invoke<Response>("mado_invoke", { request }),
  listen: (event, handler) => listen(event, (e) => handler(e.payload as never)),
  label: () => getCurrentWebview().label,
};
