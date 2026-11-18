<script lang="ts">
  // Back, forward and reload (or stop while loading) for the active tab
  // (decisions/09-ui.md §2).
  import type { CoreStore } from "../core/store.svelte";
  import Icon from "../design/Icon.svelte";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();
  const tab = $derived.by(() => {
    const id = store.state?.windows[windowId]?.activeTabId;
    return id ? store.state?.tabs[id] : undefined;
  });
  const loading = $derived(tab?.progress != null && tab.progress < 1);
  const call = (cmd: Parameters<CoreStore["client"]["call"]>[0]) =>
    store.client.call(cmd).catch(() => {});
</script>

{#if tab}
  <div class="nav">
    <button
      type="button"
      class="icon"
      aria-label={t("nav.back")}
      title={t("nav.back")}
      disabled={!tab.canGoBack}
      onclick={() => call({ type: "tab.back", tabId: tab.id })}><Icon name="back" /></button
    >
    <button
      type="button"
      class="icon"
      aria-label={t("nav.forward")}
      title={t("nav.forward")}
      disabled={!tab.canGoForward}
      onclick={() => call({ type: "tab.forward", tabId: tab.id })}><Icon name="forward" /></button
    >
    {#if loading}
      <button
        type="button"
        class="icon"
        aria-label={t("nav.stop")}
        title={t("nav.stop")}
        onclick={() => call({ type: "tab.stop", tabId: tab.id })}><Icon name="stop" /></button
      >
    {:else}
      <button
        type="button"
        class="icon"
        aria-label={t("nav.reload")}
        title={t("nav.reload")}
        onclick={(e) => call({ type: "tab.reload", tabId: tab.id, bypassCache: e.shiftKey })}
        ><Icon name="reload" /></button
      >
    {/if}
  </div>
{/if}

<style>
  .nav {
    display: flex;
    gap: var(--space-1);
  }
  .icon:disabled {
    opacity: 0.3;
  }
</style>
