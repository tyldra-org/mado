<script lang="ts">
  // The overlay webview: menus, popups and prompts from M2. It is hidden
  // until something opens in it, so it draws nothing yet.
  import type { CoreStore } from "../core/store.svelte";

  let { store }: { store: CoreStore } = $props();
  const overlay = $derived(
    store.state?.windows
      ? Object.values(store.state.windows).find((w) => w.overlay)?.overlay
      : null,
  );
</script>

{#if overlay}
  <div data-kind={overlay.kind}></div>
{/if}
