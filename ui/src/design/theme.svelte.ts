// Applies `appearance.theme` and `appearance.density` to the page's root
// element (decisions/09-ui.md §1), so tokens.css picks the right values.
import type { CoreStore } from "../core/store.svelte";

export function followTheme(store: CoreStore, root: HTMLElement = document.documentElement) {
  return $effect.root(() => {
    $effect(() => {
      const values = store.state?.settings.values ?? {};
      root.dataset.theme = String(values["appearance.theme"] ?? "system");
      root.dataset.density = String(values["appearance.density"] ?? "comfortable");
    });
  });
}
