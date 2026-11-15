<script lang="ts" module>
  export type Node = {
    id: string;
    kind: "folder" | "link" | "separator";
    title: string;
    url: string | null;
    root: string | null;
    children?: Node[];
  };
</script>

<script lang="ts">
  import type { CoreStore } from "../../core/store.svelte";
  import { t } from "../../i18n";
  import Self from "./BookmarkNode.svelte";
  import { openUrl } from "./open";

  let { node, store, onchange }: { node: Node; store: CoreStore; onchange: () => void } = $props();

  async function remove() {
    await store.client.call({ type: "bookmark.delete", ids: [node.id] }).catch(() => {});
    onchange();
  }
</script>

{#if node.kind === "folder"}
  <li>
    <strong>{node.root ? t(`bookmarks.root.${node.root}`) : node.title}</strong>
    <ul>
      {#each node.children ?? [] as child (child.id)}
        <Self node={child} {store} {onchange} />
      {/each}
    </ul>
  </li>
{:else if node.kind === "link" && node.url}
  <li>
    <button type="button" onclick={() => node.url && openUrl(store, node.url)}
      >{node.title || node.url}</button
    >
    <button
      type="button"
      aria-label={t("internal.bookmarks.remove", { title: node.title })}
      onclick={remove}>×</button
    >
  </li>
{/if}
