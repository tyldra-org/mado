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

<main>
  <h1>{t("internal.newtab.title")}</h1>
  <form onsubmit={search}>
    <input type="search" aria-label={t("internal.newtab.search")} bind:value={query} />
  </form>
  {#if recent.length}
    <h2>{t("internal.newtab.recent")}</h2>
    <ul>
      {#each recent as b (b.id)}
        <li>
          <button type="button" onclick={() => b.url && openUrl(store, b.url)}
            >{b.title || b.url}</button
          >
        </li>
      {/each}
    </ul>
  {/if}
</main>
