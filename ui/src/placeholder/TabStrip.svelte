<script lang="ts">
  // Tab strip (horizontal) and sidebar (vertical) with pinned tabs, groups,
  // drag and drop and multi-select. Placeholder until the design pass (M6):
  // correct and accessible, no visual decisions (spec 04 §2, §5, §10, §11).
  import type { CoreStore } from "../core/store.svelte";
  import type { GroupId } from "../generated/GroupId";
  import type { TabId } from "../generated/TabId";
  import type { TabState } from "../generated/TabState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  const win = $derived(store.state?.windows[windowId]);
  const tabs = $derived(
    (win?.tabOrder ?? []).map((id) => store.state?.tabs[id]).filter((x): x is TabState => !!x),
  );
  const groups = $derived(store.state?.groups ?? {});
  const layout = $derived(win?.layout ?? "vertical");

  // Multi-select is view state: it lives here, never in the core.
  let selected = $state<TabId[]>([]);
  let anchor = $state<TabId | null>(null);
  let hover = $state<{ tabId: TabId; title: string; url: string; sleeping: boolean } | null>(null);
  let hoverTimer = 0;

  const call = (cmd: Parameters<CoreStore["client"]["call"]>[0]) =>
    store.client.call(cmd).catch(() => {});

  function label(tab: TabState): string {
    return tab.customTitle ?? (tab.title || tab.url);
  }

  function choose(event: MouseEvent, id: TabId) {
    if (event.metaKey) {
      selected = selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id];
      anchor = id;
      return;
    }
    if (event.shiftKey && anchor) {
      const order = tabs.map((x) => x.id);
      const [a = 0, b = 0] = [order.indexOf(anchor), order.indexOf(id)].sort((x, y) => x - y);
      selected = order.slice(a, b + 1);
      return;
    }
    selected = [id];
    anchor = id;
    void call({ type: "tab.activate", tabId: id });
  }

  const targets = (id: TabId) => (selected.includes(id) ? selected : [id]);

  function onKey(event: KeyboardEvent, id: TabId) {
    const order = tabs.map((x) => x.id);
    const at = order.indexOf(id);
    const next = (step: number) => order[(at + step + order.length) % order.length];
    const forward = layout === "vertical" ? "ArrowDown" : "ArrowRight";
    const back = layout === "vertical" ? "ArrowUp" : "ArrowLeft";
    if (event.key === forward || event.key === back) {
      event.preventDefault();
      const to = next(event.key === forward ? 1 : -1);
      if (to) document.getElementById(`tab-${to}`)?.focus();
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      void call({ type: "tab.activate", tabId: id });
    } else if (event.key === "Delete" || event.key === "Backspace") {
      event.preventDefault();
      void call({ type: "tab.close", tabIds: targets(id) });
    }
  }

  // Drag and drop: a Mado tab payload, so another window of this profile
  // can take it (another profile's process asks first, in M2-T09).
  const MIME = "application/x-mado-tab";

  function onDragStart(event: DragEvent, tab: TabState) {
    const ids = targets(tab.id);
    event.dataTransfer?.setData(
      MIME,
      JSON.stringify({ profileId: store.state?.profile.id, windowId, tabIds: ids, url: tab.url }),
    );
    event.dataTransfer?.setData("text/uri-list", tab.url);
    if (event.dataTransfer) event.dataTransfer.effectAllowed = "move";
  }

  function onDrop(event: DragEvent, index: number, group?: GroupId | null) {
    const raw = event.dataTransfer?.getData(MIME);
    if (!raw) return;
    event.preventDefault();
    const payload = JSON.parse(raw) as { profileId: string; windowId: WindowId; tabIds: TabId[] };
    if (payload.profileId !== store.state?.profile.id) return;
    void call({
      type: "tab.move",
      tabIds: payload.tabIds,
      toWindowId: windowId,
      toIndex: index,
      // Absent (undefined) lets the core apply the drop rules.
      toGroupId: group,
    });
  }

  function onDragEnd(event: DragEvent) {
    // Dropped outside every Mado window: a new window there (spec 04 §2).
    if (event.dataTransfer?.dropEffect === "none" && event.screenX > 0) {
      const ids = selected.length ? selected : [];
      if (ids.length)
        void call({
          type: "tab.toNewWindow",
          tabIds: ids,
          bounds: { x: event.screenX, y: event.screenY, width: 0, height: 0, screen: "" },
        });
    }
  }

  function showHover(id: TabId) {
    clearTimeout(hoverTimer);
    hoverTimer = window.setTimeout(async () => {
      const info = (await store.client
        .call({ type: "tab.hoverInfo", tabId: id })
        .catch(() => null)) as {
        title: string;
        url: string;
        sleeping: boolean;
      } | null;
      if (info) hover = { tabId: id, ...info };
    }, 600);
  }

  function hideHover() {
    clearTimeout(hoverTimer);
    hover = null;
  }

  // Groups render as a header followed by their contiguous tabs.
  const rows = $derived.by(() => {
    const out: ({ kind: "group"; id: GroupId } | { kind: "tab"; tab: TabState; index: number })[] =
      [];
    let current: GroupId | null = null;
    tabs.forEach((tab, index) => {
      const g = tab.groupId ?? null;
      if (g && g !== current) out.push({ kind: "group", id: g });
      current = g;
      const collapsed = g ? groups[g]?.collapsed : false;
      if (!collapsed || tab.id === win?.activeTabId) out.push({ kind: "tab", tab, index });
    });
    return out;
  });
</script>

<nav class="tabs" data-layout={layout} aria-label={t("tabs.label")}>
  <div role="tablist" aria-orientation={layout === "vertical" ? "vertical" : "horizontal"}>
    {#each rows as row (row.kind === "group" ? `g-${row.id}` : row.tab.id)}
      {#if row.kind === "group"}
        {@const group = groups[row.id]}
        <div class="group" role="presentation" data-color={group?.color}>
          <button
            type="button"
            aria-expanded={!group?.collapsed}
            onclick={() =>
              call({
                type: "group.update",
                groupId: row.id,
                title: null,
                color: null,
                collapsed: !group?.collapsed,
              })}
          >
            {group?.title || t("tabs.unnamedGroup")}
          </button>
          <button type="button" onclick={() => call({ type: "group.close", groupId: row.id })}>
            {t("tabs.closeGroup")}
          </button>
          <button type="button" onclick={() => call({ type: "group.ungroup", groupId: row.id })}>
            {t("tabs.ungroup")}
          </button>
        </div>
      {:else}
        {@const tab = row.tab}
        <div
          class="tab"
          role="presentation"
          data-pinned={tab.pinned}
          data-life={tab.life}
          ondragover={(e) => e.preventDefault()}
          ondrop={(e) => onDrop(e, row.index)}
        >
          <button
            id={`tab-${tab.id}`}
            type="button"
            role="tab"
            draggable="true"
            aria-selected={tab.id === win?.activeTabId}
            aria-describedby={hover?.tabId === tab.id ? "hover-card" : undefined}
            data-selected={selected.includes(tab.id)}
            tabindex={tab.id === win?.activeTabId ? 0 : -1}
            onclick={(e) => choose(e, tab.id)}
            onkeydown={(e) => onKey(e, tab.id)}
            ondragstart={(e) => onDragStart(e, tab)}
            ondragend={onDragEnd}
            onmouseenter={() => showHover(tab.id)}
            onmouseleave={hideHover}
          >
            {#if tab.favicon}<img src={tab.favicon} alt="" width="16" height="16" />{/if}
            <span>{tab.pinned ? "" : label(tab)}</span>
            {#if tab.pinned}<span class="visually-hidden">{label(tab)}</span>{/if}
            {#if tab.audio.playing}<span>{t(tab.audio.muted ? "tabs.muted" : "tabs.playing")}</span
              >{/if}
            {#if tab.life === "sleeping"}<span class="visually-hidden">{t("tabs.sleeping")}</span
              >{/if}
          </button>
          {#if tab.audio.playing || tab.audio.muted}
            <button
              type="button"
              onclick={() =>
                call({ type: "tab.mute", tabIds: targets(tab.id), muted: !tab.audio.muted })}
            >
              {t(tab.audio.muted ? "tabs.unmute" : "tabs.mute")}
            </button>
          {/if}
          <button
            type="button"
            onclick={() =>
              call({ type: tab.pinned ? "tab.unpin" : "tab.pin", tabIds: targets(tab.id) })}
          >
            {t(tab.pinned ? "tabs.unpin" : "tabs.pin")}
          </button>
          <button
            type="button"
            aria-label={t("tabs.closeTab", { title: label(tab) })}
            onclick={() => call({ type: "tab.close", tabIds: targets(tab.id) })}
          >
            ×
          </button>
        </div>
      {/if}
    {/each}
    <div
      class="end"
      role="presentation"
      ondragover={(e) => e.preventDefault()}
      ondrop={(e) => onDrop(e, tabs.length, null)}
    ></div>
  </div>
  <div class="actions">
    <button
      type="button"
      onclick={() =>
        call({
          type: "tab.new",
          windowId,
          url: null,
          position: null,
          background: false,
          openerId: null,
        })}>{t("tabs.new")}</button
    >
    {#if selected.length > 1}
      <button
        type="button"
        onclick={() => call({ type: "group.create", tabIds: selected, title: null, color: null })}
        >{t("tabs.group")}</button
      >
      <button
        type="button"
        onclick={() => call({ type: "tab.toNewWindow", tabIds: selected, bounds: null })}
        >{t("tabs.toNewWindow")}</button
      >
    {/if}
    <button type="button" onclick={() => call({ type: "session.reopenClosed" })}
      >{t("tabs.reopenClosed")}</button
    >
  </div>
  {#if hover}
    <div id="hover-card" role="tooltip">
      <strong>{hover.title}</strong>
      <span>{hover.url}</span>
      {#if hover.sleeping}<span>{t("tabs.sleeping")}</span>{/if}
    </div>
  {/if}
</nav>

<style>
  .tabs[data-layout="vertical"] [role="tablist"] {
    display: flex;
    flex-direction: column;
    width: 240px;
  }
  .tabs[data-layout="horizontal"] [role="tablist"] {
    display: flex;
    flex-direction: row;
    overflow-x: auto;
  }
  .tab {
    display: flex;
  }
  .end {
    min-height: 8px;
    flex: 1;
  }
  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
