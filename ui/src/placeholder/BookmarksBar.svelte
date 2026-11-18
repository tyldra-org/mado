<script lang="ts">
  // The bookmarks bar and the ⌘D dialog (spec 12 §3). Placeholder until the
  // design pass (M6). Bookmarks are not in the state document: the bar
  // fetches them again whenever `bookmarksRev` changes.
  import { onAction } from "../core/actions";
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import Icon from "../design/Icon.svelte";
  import { portal } from "../design/portal";
  import { t } from "../i18n";

  type Node = {
    id: string;
    kind: "folder" | "link" | "separator";
    title: string;
    url: string | null;
    root: string | null;
    children?: Node[];
  };

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  const shown = $derived(store.state?.settings.values["appearance.bookmarksBar"] === true);
  const rev = $derived(store.state?.bookmarksRev ?? 0);
  const activeTab = $derived.by(() => {
    const id = store.state?.windows[windowId]?.activeTabId;
    return id ? store.state?.tabs[id] : undefined;
  });

  let roots = $state<Node[]>([]);
  let dialog = $state<{ id: string | null; title: string; parentId: string } | null>(null);

  $effect(() => {
    void rev;
    store.client
      .call({ type: "bookmark.tree", parentId: null })
      .then((t) => {
        roots = t as Node[];
      })
      .catch(() => {});
  });

  const bar = $derived(roots.find((r) => r.root === "bar"));
  const folders = $derived.by(() => {
    const out: { id: string; label: string }[] = [];
    const walk = (nodes: Node[], depth: number) => {
      for (const n of nodes) {
        if (n.kind !== "folder") continue;
        out.push({ id: n.id, label: `${"  ".repeat(depth)}${n.title}` });
        walk(n.children ?? [], depth + 1);
      }
    };
    walk(roots, 0);
    return out;
  });

  const call = (cmd: Parameters<CoreStore["client"]["call"]>[0]) => store.client.call(cmd);

  // ⌘D and ⇧⌘D from the menu (12 §3).
  $effect(() =>
    onAction(store, (action) => {
      if (action === "bookmarkTab") void bookmarkThisTab();
      if (action === "bookmarkAllTabs")
        void call({
          type: "bookmark.addAllTabs",
          windowId,
          folderName: t("bookmarks.allTabsFolder"),
        }).catch(() => {});
    }),
  );

  function open(url: string | null) {
    if (url)
      void call({
        type: "tab.new",
        windowId,
        url,
        position: null,
        background: false,
        openerId: null,
      }).catch(() => {});
  }

  /** ⌘D: edit the existing bookmark, or add one in the last folder used. */
  async function bookmarkThisTab() {
    const tab = activeTab;
    if (!tab) return;
    const existing = (await call({ type: "bookmark.ofUrl", url: tab.url }).catch(
      () => [],
    )) as (Node & {
      parentId: string;
    })[];
    if (existing[0]) {
      dialog = { id: existing[0].id, title: existing[0].title, parentId: existing[0].parentId };
      return;
    }
    const added = (await call({
      type: "bookmark.add",
      parentId: null,
      title: tab.title || tab.url,
      url: tab.url,
      index: null,
    }).catch(() => null)) as { id: string } | null;
    if (!added) return;
    const now = (await call({ type: "bookmark.ofUrl", url: tab.url }).catch(() => [])) as (Node & {
      parentId: string;
    })[];
    dialog = { id: added.id, title: tab.title || tab.url, parentId: now[0]?.parentId ?? "" };
  }

  async function done() {
    if (!dialog?.id) return;
    await call({ type: "bookmark.update", id: dialog.id, title: dialog.title, url: null }).catch(
      () => {},
    );
    await call({
      type: "bookmark.move",
      id: dialog.id,
      parentId: dialog.parentId,
      index: 4294967295,
    }).catch(() => {});
    dialog = null;
  }

  async function remove() {
    if (dialog?.id) await call({ type: "bookmark.delete", ids: [dialog.id] }).catch(() => {});
    dialog = null;
  }
</script>

<button
  type="button"
  class="icon"
  aria-label={t("bookmarks.add")}
  title={t("bookmarks.add")}
  onclick={bookmarkThisTab}
  disabled={!activeTab}><Icon name="star" /></button
>

{#if dialog}
  <div
    class="popover card floating"
    role="dialog"
    aria-labelledby="bookmark-title"
    {@attach portal("popovers")}
  >
    <h2 id="bookmark-title">{t("bookmarks.dialogTitle")}</h2>
    <label>
      {t("bookmarks.name")}
      <input bind:value={dialog.title} />
    </label>
    <label>
      {t("bookmarks.folder")}
      <select bind:value={dialog.parentId}>
        {#each folders as f (f.id)}
          <option value={f.id}>{f.label}</option>
        {/each}
      </select>
    </label>
    <button type="button" onclick={remove}>{t("bookmarks.remove")}</button>
    <button type="button" onclick={done}>{t("bookmarks.done")}</button>
  </div>
{/if}

{#if shown && bar}
  <nav class="bar" aria-label={t("bookmarks.bar")} {@attach portal("bookmarks-bar")}>
    <ul>
      {#each bar.children ?? [] as node (node.id)}
        <li>
          {#if node.kind === "link"}
            <button type="button" title={node.url ?? ""} onclick={() => open(node.url)}
              >{node.title || node.url}</button
            >
          {:else if node.kind === "folder"}
            <button
              type="button"
              onclick={() =>
                call({
                  type: "bookmark.openFolder",
                  id: node.id,
                  disposition: "newTabs",
                  windowId,
                }).catch(() => {})}>{t("bookmarks.openFolder", { title: node.title })}</button
            >
          {:else}
            <hr />
          {/if}
        </li>
      {/each}
    </ul>
  </nav>
{/if}
