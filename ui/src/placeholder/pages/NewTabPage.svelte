<script lang="ts">
  // mado://newtab: a search box and recent bookmarks.
  import type { CoreStore } from "../../core/store.svelte";
  import { t } from "../../i18n";
  import { openUrl } from "./open";

  type Node = { id: string; title: string; url: string | null };
  let { store }: { store: CoreStore } = $props();
  let query = $state("");
  let recent = $state<Node[]>([]);

  $effect(() => {
    void store.client
      .call({ type: "bookmark.recent", limit: 8 })
      .then((r) => (recent = (r as Node[]).filter((n) => n.url)))
      .catch(() => {});
  });

  async function search(e: SubmitEvent) {
    e.preventDefault();
    if (query.trim()) await openUrl(store, query.trim());
  }
</script>

<main class="newtab">
  <h1 class="visually-hidden">{t("internal.newtab.title")}</h1>
  <form onsubmit={search}>
    <!-- svelte-ignore a11y_autofocus -->
    <input
      type="search"
      autofocus
      placeholder={t("internal.newtab.search")}
      aria-label={t("internal.newtab.search")}
      bind:value={query}
    />
  </form>
  {#if recent.length}
    <h2>{t("internal.newtab.recent")}</h2>
    <ul>
      {#each recent as b (b.id)}
        <li>
          <button type="button" class="tile" onclick={() => b.url && openUrl(store, b.url)}>
            <span class="tile-icon" aria-hidden="true"
              >{(b.title || b.url || "?").charAt(0).toUpperCase()}</span
            >
            <span class="tile-name">{b.title || b.url}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</main>

<style>
  .newtab {
    min-height: 100vh;
    justify-content: flex-start;
    align-items: center;
    padding-top: 30vh !important;
  }
  form {
    width: min(560px, 100%);
  }
  .newtab input {
    min-height: 44px !important;
    border-radius: var(--radius-card) !important;
    font-size: var(--text-title);
    text-align: center;
  }
  .newtab input:focus {
    text-align: left;
    background: var(--surface-raised) !important;
    border-color: var(--accent) !important;
  }
  .newtab h2 {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .newtab ul {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--space-5);
    max-width: 560px;
    margin-top: var(--space-6);
  }
  .newtab li {
    min-height: 0 !important;
  }
  .tile {
    flex-direction: column;
    gap: var(--space-3);
    width: 80px;
    height: auto !important;
    padding: var(--space-4) var(--space-2) !important;
    border: 0 !important;
    background: transparent !important;
  }
  .tile:hover .tile-icon {
    background: var(--fill-pressed);
  }
  .tile-icon {
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    border-radius: var(--radius-card);
    background: var(--fill);
    font-size: var(--text-title);
    font-weight: var(--weight-strong);
    color: var(--text);
  }
  .tile-name {
    max-width: 76px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-small);
    font-weight: var(--weight-regular);
    color: var(--text-secondary);
  }
</style>
