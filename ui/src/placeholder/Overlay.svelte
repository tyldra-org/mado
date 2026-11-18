<script lang="ts">
  // The overlay webview (spec 01 §5): sized to what it shows and drawn over
  // the page. It shows the command bar (decisions/09-ui.md §3); a click
  // outside the card or Escape closes it.
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import AddressBar from "./AddressBar.svelte";

  let { store }: { store: CoreStore } = $props();
  const windowId = $derived(store.transport.label().replace(/^overlay-/, "") as WindowId);
  const overlay = $derived(store.state?.windows[windowId]?.overlay ?? null);
  const close = () => store.client.call({ type: "window.overlayClose", windowId }).catch(() => {});
</script>

<svelte:window onblur={() => overlay && close()} />

{#if overlay}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="backdrop"
    data-kind={overlay.kind}
    onclick={(e) => e.target === e.currentTarget && close()}
  >
    {#if overlay.kind === "commandBar"}
      <div class="command card floating">
        {#key overlay.id}
          <AddressBar {store} {windowId} mode="palette" />
        {/key}
      </div>
    {/if}
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    padding: 2px 2px 24px;
  }
  .command {
    border-radius: var(--radius-sheet);
    animation: enter var(--duration-panel) var(--ease-out);
  }
</style>
