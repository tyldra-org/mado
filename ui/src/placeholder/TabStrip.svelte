<script lang="ts">
  // Tab strip (horizontal) and sidebar (vertical) with pinned tabs, groups,
  // drag and drop and multi-select. Placeholder until the design pass (M6):
  // correct and accessible, no visual decisions (spec 04 §2, §5, §10, §11).
  import type { CoreStore } from "../core/store.svelte";
  import type { GroupId } from "../generated/GroupId";
  import type { TabId } from "../generated/TabId";
  import type { TabState } from "../generated/TabState";
  import type { WindowId } from "../generated/WindowId";
  import Icon from "../design/Icon.svelte";
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

  function initial(tab: TabState): string {
    try {
      return new URL(tab.url).hostname.replace(/^www\./, "").charAt(0) || "·";
    } catch {
      return "·";
    }
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
        <div
          class="group"
          role="presentation"
          data-color={group?.color}
          data-agent={!!group?.agentTaskId}
          data-collapsed={!!group?.collapsed}
        >
          <button
            type="button"
            class="group-name"
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
            {#if group?.agentTaskId}<Icon name="sparkle" size={12} />{:else}<span class="dot"
              ></span>{/if}
            <span class="name">{group?.title || t("tabs.unnamedGroup")}</span>
            <span class="chevron"><Icon name="chevronDown" size={12} /></span>
          </button>
          <span class="hover-actions">
            <button
              type="button"
              class="icon"
              aria-label={t("tabs.ungroup")}
              title={t("tabs.ungroup")}
              onclick={() => call({ type: "group.ungroup", groupId: row.id })}
              ><Icon name="more" size={14} /></button
            >
            <button
              type="button"
              class="icon"
              aria-label={t("tabs.closeGroup")}
              title={t("tabs.closeGroup")}
              onclick={() => call({ type: "group.close", groupId: row.id })}
              ><Icon name="close" size={14} /></button
            >
          </span>
        </div>
      {:else}
        {@const tab = row.tab}
        <div
          class="tab"
          role="presentation"
          data-pinned={tab.pinned}
          data-life={tab.life}
          data-active={tab.id === win?.activeTabId}
          data-grouped={!!tab.groupId}
          data-color={tab.groupId ? groups[tab.groupId]?.color : undefined}
          ondragover={(e) => e.preventDefault()}
          ondrop={(e) => onDrop(e, row.index)}
        >
          <button
            id={`tab-${tab.id}`}
            type="button"
            role="tab"
            class="tab-main"
            draggable="true"
            title={tab.pinned ? label(tab) : undefined}
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
            <span class="favicon">
              <span class="letter" data-initial={initial(tab)} aria-hidden="true"></span>
              {#if tab.favicon}<img
                  src={tab.favicon}
                  alt=""
                  width="16"
                  height="16"
                  onerror={(e) => e.currentTarget.remove()}
                />{/if}
            </span>
            <span class="title">{tab.pinned ? "" : label(tab)}</span>
            {#if tab.pinned}<span class="visually-hidden">{label(tab)}</span>{/if}
            {#if tab.audio.playing}<span class="visually-hidden"
                >{t(tab.audio.muted ? "tabs.muted" : "tabs.playing")}</span
              >{/if}
            {#if tab.life === "sleeping"}<span class="visually-hidden">{t("tabs.sleeping")}</span
              >{/if}
          </button>
          {#if tab.audio.playing || tab.audio.muted}
            <button
              type="button"
              class="icon audio"
              aria-label={t(tab.audio.muted ? "tabs.unmute" : "tabs.mute")}
              title={t(tab.audio.muted ? "tabs.unmute" : "tabs.mute")}
              onclick={() =>
                call({ type: "tab.mute", tabIds: targets(tab.id), muted: !tab.audio.muted })}
              ><Icon name={tab.audio.muted ? "muted" : "speaker"} size={14} /></button
            >
          {/if}
          {#if !tab.pinned}
            <span class="hover-actions">
              <button
                type="button"
                class="icon"
                aria-label={t("tabs.pin")}
                title={t("tabs.pin")}
                onclick={() => call({ type: "tab.pin", tabIds: targets(tab.id) })}
                ><Icon name="pin" size={14} /></button
              >
              <button
                type="button"
                class="icon"
                aria-label={t("tabs.closeTab", { title: label(tab) })}
                title={t("tabs.closeTab", { title: label(tab) })}
                onclick={() => call({ type: "tab.close", tabIds: targets(tab.id) })}
                ><Icon name="close" size={14} /></button
              >
            </span>
          {:else}
            <button
              type="button"
              class="visually-hidden"
              onclick={() => call({ type: "tab.unpin", tabIds: targets(tab.id) })}
              >{t("tabs.unpin")}</button
            >
            <button
              type="button"
              class="visually-hidden"
              aria-label={t("tabs.closeTab", { title: label(tab) })}
              onclick={() => call({ type: "tab.close", tabIds: targets(tab.id) })}
            ></button>
          {/if}
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
      class="new-tab"
      onclick={() =>
        call({
          type: "tab.new",
          windowId,
          url: null,
          position: null,
          background: false,
          openerId: null,
        })}><Icon name="plus" size={14} /><span class="label">{t("tabs.new")}</span></button
    >
    {#if selected.length > 1}
      <button
        type="button"
        class="quiet"
        onclick={() => call({ type: "group.create", tabIds: selected, title: null, color: null })}
        >{t("tabs.group")}</button
      >
      <button
        type="button"
        class="quiet"
        onclick={() => call({ type: "tab.toNewWindow", tabIds: selected, bounds: null })}
        >{t("tabs.toNewWindow")}</button
      >
    {/if}
    <button
      type="button"
      class="icon reopen"
      aria-label={t("tabs.reopenClosed")}
      title={t("tabs.reopenClosed")}
      onclick={() => call({ type: "session.reopenClosed" })}
      ><Icon name="history" size={14} /></button
    >
  </div>
  {#if hover}
    <div id="hover-card" class="card floating" role="tooltip">
      <strong>{hover.title}</strong>
      <span class="url">{hover.url}</span>
      {#if hover.sleeping}<span class="url">{t("tabs.sleeping")}</span>{/if}
    </div>
  {/if}
</nav>

<style>
  .tabs {
    display: flex;
    flex-direction: column;
    min-height: 0;
    color: var(--text-on-chrome);
  }
  [role="tablist"] {
    display: flex;
    gap: 1px;
  }

  /* Vertical: pinned tiles in a grid, then rows. */
  .tabs[data-layout="vertical"] {
    flex: 1;
  }
  .tabs[data-layout="vertical"] [role="tablist"] {
    flex-flow: row wrap;
    align-content: flex-start;
    flex: 1;
    overflow-y: auto;
    padding: 0 var(--space-4);
  }
  .tabs[data-layout="vertical"] .tab,
  .tabs[data-layout="vertical"] .group,
  .tabs[data-layout="vertical"] .end {
    flex: 1 1 100%;
  }
  .tabs[data-layout="vertical"] .tab[data-pinned="true"] {
    flex: 0 0 auto;
    margin: 0 var(--space-2) var(--space-4) 0;
  }
  .tabs[data-layout="vertical"] .tab[data-pinned="true"] .tab-main {
    width: 52px;
    height: 36px;
    justify-content: center;
    border-radius: var(--radius-control);
    background: var(--fill);
  }
  .tabs[data-layout="vertical"] .tab[data-grouped="true"] {
    padding-left: var(--space-5);
  }

  .tab,
  .group {
    position: relative;
    display: flex;
    align-items: center;
    min-width: 0;
    border-radius: var(--radius-control);
  }
  .tab-main,
  .group-name {
    flex: 1;
    min-width: 0;
    height: var(--tab-height);
    justify-content: flex-start;
    gap: var(--space-4);
    padding: 0 var(--space-4);
    border: 0;
    border-radius: var(--radius-control);
    background: transparent;
    font-weight: var(--weight-regular);
    color: inherit;
  }
  .tab:hover .tab-main:not([aria-selected="true"]) {
    background: var(--fill-hover);
  }
  .tab-main[aria-selected="true"] {
    background: var(--chrome-active);
    box-shadow:
      0 0 0 1px var(--line),
      0 1px 2px oklch(0 0 0 / 0.06);
  }
  .tab-main[data-selected="true"]:not([aria-selected="true"]) {
    background: var(--fill-pressed);
  }
  .tab[data-life="sleeping"] .title {
    color: var(--text-secondary);
  }
  .title,
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .favicon {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    flex: none;
  }
  .favicon {
    position: relative;
  }
  .favicon img {
    position: absolute;
    inset: 0;
    border-radius: 3px;
    background: var(--chrome);
  }
  .letter::before {
    content: attr(data-initial);
  }
  .letter {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    border-radius: 4px;
    background: var(--fill-pressed);
    font-size: 10px;
    font-weight: var(--weight-strong);
    text-transform: uppercase;
  }
  .hover-actions {
    position: absolute;
    right: var(--space-1);
    display: none;
    gap: 0;
  }
  .tab:hover .hover-actions,
  .tab:focus-within .hover-actions,
  .group:hover .hover-actions,
  .group:focus-within .hover-actions {
    display: flex;
  }
  .hover-actions .icon,
  .audio {
    width: 22px;
    min-height: 22px;
  }
  .audio {
    margin-right: var(--space-2);
  }
  .tab:hover .audio {
    margin-right: 50px;
  }

  .group-name {
    gap: var(--space-3);
    height: 26px;
    margin-top: var(--space-4);
    font-size: var(--text-small);
    font-weight: var(--weight-strong);
    color: var(--text-secondary);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--group, var(--text-tertiary));
  }
  .group[data-agent="true"] .group-name {
    color: var(--accent);
  }
  .chevron {
    margin-left: auto;
    opacity: 0;
    transition: transform var(--duration-press) var(--ease-out);
  }
  .group:hover .chevron {
    opacity: 1;
  }
  .group:hover .chevron {
    margin-right: 46px;
  }
  .group[data-collapsed="true"] .chevron {
    transform: rotate(-90deg);
  }
  [data-color="grey"] {
    --group: oklch(0.6 0.01 250);
  }
  [data-color="blue"] {
    --group: oklch(0.62 0.15 255);
  }
  [data-color="red"] {
    --group: oklch(0.62 0.19 25);
  }
  [data-color="yellow"] {
    --group: oklch(0.8 0.15 90);
  }
  [data-color="green"] {
    --group: oklch(0.65 0.15 150);
  }
  [data-color="pink"] {
    --group: oklch(0.7 0.15 350);
  }
  [data-color="purple"] {
    --group: oklch(0.6 0.17 300);
  }
  [data-color="cyan"] {
    --group: oklch(0.72 0.11 210);
  }
  [data-color="orange"] {
    --group: oklch(0.7 0.16 55);
  }

  .end {
    min-height: 24px;
    flex: 1;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4);
  }
  .new-tab {
    flex: 1;
    justify-content: flex-start;
    gap: var(--space-4);
    padding: 0 var(--space-4);
    height: var(--tab-height);
    border: 0;
    background: transparent;
    font-weight: var(--weight-regular);
    color: var(--text-secondary);
  }
  .new-tab:hover {
    background: var(--fill-hover) !important;
    color: var(--text-on-chrome);
  }
  .tabs[data-layout="vertical"] .actions {
    order: -1;
  }

  /* Horizontal: one row of shrinking tabs. */
  .tabs[data-layout="horizontal"] {
    flex-direction: row;
    align-items: center;
    flex: 1;
    min-width: 0;
    height: 100%;
  }
  .tabs[data-layout="horizontal"] [role="tablist"] {
    flex: 0 1 auto;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs[data-layout="horizontal"] .tab {
    flex: 0 1 220px;
    min-width: 80px;
  }
  .tabs[data-layout="horizontal"] .tab[data-pinned="true"] {
    flex: 0 0 36px;
    min-width: 36px;
  }
  .tabs[data-layout="horizontal"] .tab[data-pinned="true"] .tab-main {
    justify-content: center;
    padding: 0;
  }
  .tabs[data-layout="horizontal"] .group {
    flex: 0 0 auto;
    max-width: 140px;
  }
  .tabs[data-layout="horizontal"] .group-name {
    margin-top: 0;
    height: 22px;
    padding: 0 var(--space-4);
    border-radius: 11px;
    background: color-mix(in oklch, var(--group, var(--text-tertiary)), transparent 82%);
    color: var(--text-on-chrome);
  }
  .tabs[data-layout="horizontal"] .chevron {
    display: none;
  }
  .tabs[data-layout="horizontal"] .group:hover .hover-actions {
    display: none;
  }
  .tabs[data-layout="horizontal"] .new-tab {
    flex: 0 0 auto;
    width: var(--tab-height);
    justify-content: center;
    padding: 0;
  }
  .tabs[data-layout="horizontal"] .new-tab .label {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .tabs[data-layout="horizontal"] .end {
    min-width: 16px;
    min-height: 0;
  }

  #hover-card {
    position: absolute;
    left: var(--space-4);
    right: var(--space-4);
    bottom: 56px;
    z-index: 2;
    display: grid;
    gap: var(--space-1);
    padding: var(--space-4) var(--space-5);
    color: var(--text);
    pointer-events: none;
  }
  .tabs[data-layout="horizontal"] #hover-card {
    top: calc(var(--tab-height) + 4px);
    bottom: auto;
    right: auto;
    width: 280px;
  }
  .url {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-small);
    color: var(--text-secondary);
  }
</style>
