<script lang="ts">
  // The find bar (⌘F, ⌘G, ⇧⌘G; spec 05). Matches come from the `find` page
  // script through `state.windows[id].findBar`. Placeholder until M6.
  import { onAction } from "../core/actions";
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  const win = $derived(store.state?.windows[windowId]);
  const tabId = $derived(win?.activeTabId ?? null);
  let open = $state(false);
  let query = $state("");
  let input: HTMLInputElement | undefined = $state();

  function find(forward: boolean, close = false) {
    if (!tabId) return;
    void store.client
      .call({
        type: "tab.find",
        tabId,
        query: close ? null : query,
        forward,
        caseSensitive: false,
        close,
      })
      .catch(() => {});
  }

  function hide() {
    find(true, true);
    open = false;
  }

  $effect(() =>
    onAction(store, (action) => {
      if (action === "find") {
        open = true;
        queueMicrotask(() => input?.select());
      } else if (open && action === "findNext") find(true);
      else if (open && action === "findPrevious") find(false);
    }),
  );
</script>

{#if open}
  <div role="search">
    <input
      bind:this={input}
      type="search"
      aria-label={t("find.label")}
      bind:value={query}
      oninput={() => find(true)}
      onkeydown={(e) => {
        if (e.key === "Enter") find(!e.shiftKey);
        if (e.key === "Escape") hide();
      }}
    />
    {#if win?.findBar?.matches != null}
      <output
        >{t("find.count", {
          active: win.findBar.active ?? 0,
          matches: win.findBar.matches,
        })}</output
      >
    {/if}
    <button type="button" onclick={() => find(false)}>{t("find.previous")}</button>
    <button type="button" onclick={() => find(true)}>{t("find.next")}</button>
    <button type="button" onclick={hide}>{t("find.close")}</button>
  </div>
{/if}
