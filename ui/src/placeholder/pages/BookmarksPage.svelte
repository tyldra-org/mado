<script lang="ts">
  // mado://bookmarks: the bar and Other bookmarks as a tree.
  import type { CoreStore } from "../../core/store.svelte";
  import { t } from "../../i18n";
  import BookmarkNode from "./BookmarkNode.svelte";
  import type { Node } from "./BookmarkNode.svelte";

  let { store }: { store: CoreStore } = $props();
  let roots = $state<Node[]>([]);

  async function load() {
    roots = (await store.client
      .call({ type: "bookmark.tree", parentId: null })
      .catch(() => [])) as Node[];
  }

  $effect(() => {
    void load();
  });
</script>

<main>
  <h1>{t("internal.bookmarks.title")}</h1>
  <ul>
    {#each roots as n (n.id)}
      <BookmarkNode node={n} {store} onchange={load} />
    {/each}
  </ul>
</main>
