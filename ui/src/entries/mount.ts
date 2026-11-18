// Shared start-up for every page: mount once the core store has the state.
import "../design/tokens.css";
import "../design/base.css";
import { type Component, mount } from "svelte";
import { CoreStore } from "../core/store.svelte";
import { tauri } from "../core/transport";
import { followTheme } from "../design/theme.svelte";

export function start<P extends Record<string, unknown>>(
  component: Component<P>,
  props: (store: CoreStore) => P,
) {
  const target = document.getElementById("app");
  if (!target) throw new Error("Mado UI: #app element is missing");
  const store = new CoreStore(tauri);
  document.documentElement.dataset.surface = store.transport.label().replace(/-.*/, "");
  followTheme(store);
  mount(component, { target, props: props(store) });
  void store.start();
}
