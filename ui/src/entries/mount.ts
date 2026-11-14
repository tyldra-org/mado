// Shared start-up for every page: mount once the core store has the state.
import { type Component, mount } from "svelte";
import { CoreStore } from "../core/store.svelte";
import { tauri } from "../core/transport";

export function start<P extends Record<string, unknown>>(
  component: Component<P>,
  props: (store: CoreStore) => P,
) {
  const target = document.getElementById("app");
  if (!target) throw new Error("Mado UI: #app element is missing");
  const store = new CoreStore(tauri);
  mount(component, { target, props: props(store) });
  void store.start();
}
