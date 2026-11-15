<script lang="ts">
  // The address bar with its suggestions and security icon (spec 05).
  // Placeholder until the design pass (M6): every behaviour, no styling.
  import { onAction } from "../core/actions";
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  type Row = {
    kind: string;
    input: string;
    title: string;
    url: string | null;
    tabId: string | null;
    removable: boolean;
  };
  type Engine = { id: string; name: string; active: boolean };

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  const tab = $derived.by(() => {
    const id = store.state?.windows[windowId]?.activeTabId;
    return id ? store.state?.tabs[id] : undefined;
  });

  let text = $state("");
  let editing = $state(false);
  let rows = $state<Row[]>([]);
  let inline = $state<string | null>(null);
  let selected = $state(-1);
  let request = 0;
  let deleted = false;
  let engines = $state<Engine[] | null>(null);
  let pending = "";
  let result = $state<string | null>(null);
  let input: HTMLInputElement | undefined = $state();

  // ⌘L, F6 (01 §5.3): focus moves to the address bar with its text selected.
  $effect(() =>
    onAction(store, (action) => {
      if ((action === "openLocation" || action === "focusAddressBar") && input) {
        input.focus();
        input.select();
      }
    }),
  );

  // Remote suggestions arrive later for the latest request only.
  $effect(() => {
    let stop: (() => void) | undefined;
    void store.transport
      .listen<{ windowId: string; requestId: number; rows: Row[] }>("mado://suggest", (e) => {
        if (e.windowId === windowId && e.requestId === request && editing) rows = e.rows;
      })
      .then((s) => {
        stop = s;
      });
    return () => stop?.();
  });

  async function suggest() {
    request += 1;
    const id = request;
    const out = (await store.client
      .call({
        type: "nav.suggest",
        windowId,
        input: text,
        cursor: text.length,
        deleted,
        requestId: id,
      })
      .catch(() => null)) as { requestId: number; rows: Row[]; inline: string | null } | null;
    if (!out || id !== request) return;
    rows = out.rows;
    inline = out.inline;
    selected = -1;
  }

  async function open(
    input: string,
    source: "typed" | "suggestion",
    ctrlEnter = false,
    tabId?: string | null,
  ) {
    if (tabId) {
      await store.client.call({ type: "tab.activate", tabId }).catch(() => {});
      editing = false;
      return;
    }
    const out = (await store.client
      .call({
        type: "nav.open",
        windowId,
        tabId: tab?.id ?? null,
        input,
        disposition: "currentTab",
        source,
        ctrlEnter,
      })
      .catch(() => null)) as { needsEngine?: boolean; result?: string } | null;
    if (out?.needsEngine) {
      pending = input;
      engines = (await store.client
        .call({ type: "nav.searchEngines" })
        .catch(() => [])) as Engine[];
      return;
    }
    if (out?.result !== undefined) {
      result = out.result;
      return;
    }
    editing = false;
    rows = [];
  }

  async function pick(engine: string) {
    await store.client
      .call({ type: "settings.set", key: "search.defaultEngine", value: engine, token: null })
      .catch(() => {});
    engines = null;
    await open(pending, "typed");
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      selected = Math.min(selected + 1, rows.length - 1);
      e.preventDefault();
    } else if (e.key === "ArrowUp") {
      selected = Math.max(selected - 1, -1);
      e.preventDefault();
    } else if (e.key === "Enter") {
      const row = rows[selected];
      if (row) void open(row.input, "suggestion", false, row.tabId);
      else void open(text + (inline ?? ""), "typed", e.metaKey || e.ctrlKey);
    } else if (e.key === "Delete" && e.shiftKey && rows[selected]?.removable) {
      const url = rows[selected]?.url;
      if (url)
        void store.client.call({ type: "nav.removeSuggestion", url }).then(suggest, () => {});
      e.preventDefault();
    } else if (e.key === "Escape") {
      editing = false;
      rows = [];
    } else {
      deleted = e.key === "Backspace" || e.key === "Delete";
    }
  }
</script>

<div class="address">
  <span class="security" data-level={tab?.security?.level ?? "internal"}
    >{t(`address.security.${tab?.security?.level ?? "internal"}`)}</span
  >
  <input
    bind:this={input}
    aria-label={t("address.label")}
    value={editing ? text : (tab?.displayUrl?.text ?? "")}
    onfocus={() => {
      editing = true;
      text = tab?.url ?? "";
    }}
    oninput={(e) => {
      text = e.currentTarget.value;
      result = null;
      void suggest();
    }}
    onkeydown={keydown}
  />
  {#if tab?.readerAvailable}
    <button
      type="button"
      aria-pressed={tab.readerActive}
      onclick={() =>
        store.client
          .call({ type: "tab.reader", tabId: tab.id, on: !tab.readerActive })
          .catch(() => {})}>{t(tab.readerActive ? "address.readerExit" : "address.reader")}</button
    >
    {#if !tab.readerActive}
      <button
        type="button"
        onclick={() =>
          store.client
            .call({
              type: "site.setReaderAlways",
              host: new URL(tab.url).hostname,
              always: true,
            })
            .catch(() => {})}>{t("address.readerAlways")}</button
      >
    {/if}
  {/if}
  {#if editing && inline}<span class="inline">{inline}</span>{/if}
  {#if result !== null}<output>{result}</output>{/if}
  {#if editing && rows.length}
    <ul role="listbox" aria-label={t("address.suggestions")}>
      {#each rows as row, i (i)}
        <li role="option" aria-selected={i === selected}>
          <button type="button" onclick={() => open(row.input, "suggestion", false, row.tabId)}>
            {row.kind === "tab" ? t("address.switchToTab", { title: row.title }) : row.title}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  {#if engines}
    <div role="dialog" aria-labelledby="engine-title">
      <h2 id="engine-title">{t("address.pickEngine")}</h2>
      {#each engines.filter((e) => e.active) as e (e.id)}
        <button type="button" onclick={() => pick(e.id)}>{e.name}</button>
      {/each}
    </div>
  {/if}
</div>
