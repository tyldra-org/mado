<script lang="ts">
  // The address bar with its suggestions and security icon (spec 05).
  // Placeholder until the design pass (M6): every behaviour, no styling.
  import { onAction } from "../core/actions";
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import Icon from "../design/Icon.svelte";
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

  // `field` edits in place (and in tests); `launcher` is the toolbar's
  // display, which opens the command bar in the overlay (decisions/09-ui.md
  // §3); `palette` is the command bar's field there.
  let {
    store,
    windowId,
    mode = "field",
  }: { store: CoreStore; windowId: WindowId; mode?: "field" | "launcher" | "palette" } = $props();

  // The command bar: centred near the top of the page area.
  function launch() {
    const page = document.querySelector("main")?.getBoundingClientRect();
    if (!page || page.width < 240) return;
    const width = Math.min(640, page.width - 32);
    const rect = {
      x: Math.round(page.x + (page.width - width) / 2),
      y: Math.round(page.y + 12),
      width: Math.round(width),
      height: Math.round(Math.min(440, page.height - 24)),
    };
    void store.client
      .call({ type: "window.overlayOpen", windowId, kind: "commandBar", id: "location", rect })
      .catch(() => {});
  }

  const close = () => store.client.call({ type: "window.overlayClose", windowId }).catch(() => {});

  // The palette starts editing the page's address, selected.
  $effect(() => {
    if (mode !== "palette" || !input) return;
    editing = true;
    text = tab?.url ?? "";
    const field = input;
    queueMicrotask(() => {
      field.value = text;
      field.focus();
      field.select();
    });
  });

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
      if (action !== "openLocation" && action !== "focusAddressBar") return;
      if (mode === "launcher") launch();
      else if (input) {
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
      if (mode === "palette") void close();
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
    if (mode === "palette") void close();
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
      if (mode === "palette") void close();
    } else {
      deleted = e.key === "Backspace" || e.key === "Delete";
    }
  }
</script>

<div class="address" data-mode={mode}>
  <span
    class="security"
    class:visually-hidden={["secure", "internal", "local"].includes(
      tab?.security?.level ?? "internal",
    )}
    data-level={tab?.security?.level ?? "internal"}
    >{t(`address.security.${tab?.security?.level ?? "internal"}`)}</span
  >
  <input
    bind:this={input}
    class:editing
    spellcheck="false"
    autocomplete="off"
    placeholder={t("address.placeholder")}
    aria-label={t("address.label")}
    value={editing ? text : (tab?.displayUrl?.text ?? "")}
    readonly={mode === "launcher"}
    onmousedown={(e) => {
      if (mode === "launcher") {
        e.preventDefault();
        launch();
      }
    }}
    onfocus={() => {
      if (mode === "launcher") return;
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
            <Icon
              name={row.kind === "tab"
                ? "sidebar"
                : row.kind === "history"
                  ? "history"
                  : row.kind === "bookmark"
                    ? "bookmark"
                    : row.kind === "search"
                      ? "search"
                      : "globe"}
              size={14}
            />
            <span class="row-title"
              >{row.kind === "tab"
                ? t("address.switchToTab", { title: row.title })
                : row.title}</span
            >
            {#if row.url && row.url !== row.title}<span class="row-url">{row.url}</span>{/if}
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

<style>
  .address {
    position: relative;
    display: flex;
    align-items: center;
    flex: 1;
    min-width: 0;
    gap: var(--space-3);
    height: 30px;
    padding: 0 var(--space-2) 0 var(--space-4);
    border-radius: var(--radius-control);
    background: var(--fill);
    transition: background-color var(--duration-press) var(--ease-out);
  }
  .address:hover {
    background: var(--fill-hover);
  }
  .address:focus-within {
    background: var(--surface-raised);
    box-shadow: 0 0 0 2px color-mix(in oklch, var(--accent), transparent 40%);
  }
  input {
    flex: 1;
    min-width: 0;
    min-height: 0 !important;
    height: 100%;
    padding: 0 !important;
    border: 0 !important;
    background: transparent !important;
    text-align: center;
    color: var(--text-on-chrome);
  }
  input.editing {
    text-align: left;
    color: var(--text);
  }
  .security {
    flex: none;
    padding: 1px var(--space-3);
    border-radius: var(--radius-small);
    font-size: var(--text-caption);
    font-weight: var(--weight-strong);
  }
  .security[data-level="notSecure"],
  .security[data-level="mixed"] {
    background: var(--warning-tint);
    color: var(--warning);
  }
  .security[data-level="dangerous"] {
    background: var(--danger-tint);
    color: var(--danger);
  }
  .address > button {
    min-height: 22px;
    padding: 0 var(--space-3);
    border: 0;
    background: transparent;
    font-size: var(--text-small);
    color: var(--text-secondary);
  }
  .address > button:hover {
    background: var(--fill-hover) !important;
  }
  .inline {
    color: var(--text-tertiary);
  }
  .address[data-mode="palette"] {
    height: 44px;
    padding: 0 var(--space-5);
    border-radius: var(--radius-card) var(--radius-card) 0 0;
    background: transparent;
    box-shadow: none !important;
    font-size: var(--text-title);
  }
  .address[data-mode="palette"] input {
    text-align: left;
    color: var(--text);
  }
  .address[data-mode="palette"] [role="listbox"] {
    position: static;
    flex-basis: 100%;
    border: 0;
    border-top: 1px solid var(--line);
    border-radius: 0;
    box-shadow: none;
    background: transparent;
  }
  .address[data-mode="palette"] {
    flex-wrap: wrap;
    height: auto;
    min-height: 44px;
  }
  .address[data-mode="launcher"] input {
    cursor: default;
  }
  [role="listbox"] {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    z-index: 3;
    padding: var(--space-2);
    border: 1px solid var(--line);
    border-radius: var(--radius-card);
    background: var(--surface-raised);
    box-shadow: var(--shadow-float);
  }
  .row-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row-url {
    margin-left: auto;
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-secondary);
    font-size: var(--text-small);
    font-weight: var(--weight-regular);
  }
  [role="option"][aria-selected="true"] .row-url {
    color: inherit;
    opacity: 0.8;
  }
  [role="option"] button {
    width: 100%;
    min-height: var(--row-height);
    gap: var(--space-4);
    justify-content: flex-start;
    border: 0;
    background: transparent;
    font-weight: var(--weight-regular);
  }
  [role="option"][aria-selected="true"] button {
    background: var(--accent);
    color: var(--accent-text);
  }
</style>
