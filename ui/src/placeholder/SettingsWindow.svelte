<script lang="ts">
  // The settings window (label `settings`, Origin::Settings). Sections and
  // rows come from the registry; search asks the core. Placeholder until M6.
  import { type SettingEntry, entries, sections, visible } from "../core/settings-index";
  import type { CoreStore } from "../core/store.svelte";
  import Icon from "../design/Icon.svelte";
  import type { IconName } from "../design/icons";
  import { t } from "../i18n";
  import Notices from "./Notices.svelte";
  import SettingRow from "./SettingRow.svelte";
  import ProfilesSection from "./ProfilesSection.svelte";
  import SiteExceptions from "./SiteExceptions.svelte";
  import ClearData from "./ClearData.svelte";
  import ImportSheet from "./ImportSheet.svelte";
  import DefaultBrowser from "./DefaultBrowser.svelte";
  import ExtensionsSection from "./ExtensionsSection.svelte";
  import BlockerSection from "./BlockerSection.svelte";
  import PasswordSources from "./PasswordSources.svelte";
  import PasswordsSection from "./PasswordsSection.svelte";
  import AiConnections from "./AiConnections.svelte";
  import AiModels from "./AiModels.svelte";
  import MemorySection from "./MemorySection.svelte";
  import AgentSetup from "./AgentSetup.svelte";
  import ControlClients from "./ControlClients.svelte";

  let { store }: { store: CoreStore } = $props();

  const icons: Record<string, IconName> = {
    general: "gear",
    appearance: "sidebar",
    tabs: "sidebar",
    search: "search",
    privacy: "shield",
    siteSettings: "globe",
    passwords: "key",
    extensions: "puzzle",
    downloads: "download",
    ai: "sparkle",
    profiles: "person",
    shortcuts: "more",
    developers: "tasks",
    features: "star",
    startup: "reload",
    languages: "globe",
    accessibility: "person",
    system: "gear",
    notifications: "bell",
  };

  let query = $state("");
  let changedOnly = $state(false);
  // The section a link asked for (`settings.html#extensions`, or the
  // `mado://settings-section` event when the window is open).
  const asked = (s: string) => (sections as string[]).includes(s);
  let section = $state(
    asked(location.hash.slice(1)) ? location.hash.slice(1) : (sections[0] ?? "general"),
  );
  $effect(() => {
    let stop: (() => void) | undefined;
    void store.transport
      .listen<string>("mado://settings-section", (s) => {
        if (asked(s)) {
          section = s;
          hits = null;
        }
      })
      .then((u) => (stop = u));
    return () => stop?.();
  });
  /** Keys from settings.search, best first; null while not searching. */
  let hits: string[] | null = $state(null);

  const values = $derived(store.state?.settings.values ?? {});
  const controller = (key: string) => {
    const id = store.state?.extensions?.controlled?.[key];
    const e = id ? store.state?.extensions?.installed.find((x) => x.id === id) : undefined;
    return e ? { id: e.id, name: e.name } : undefined;
  };
  const changed = $derived(new Set(store.state?.settings.changed ?? []));
  const features = $derived(store.state?.features ?? {});

  const rows: SettingEntry[] = $derived.by(() => {
    const shown = entries.filter(
      (e) => visible(e, features) && (!changedOnly || changed.has(e.key)),
    );
    if (hits === null) return shown.filter((e) => e.section === section);
    const byKey = new Map(shown.map((e) => [e.key, e]));
    return hits.flatMap((k) => byKey.get(k) ?? []);
  });

  async function search() {
    const text = query.trim();
    if (text === "") {
      hits = null;
      return;
    }
    const result = (await store.client.call({ type: "settings.search", query: text })) as {
      key: string;
    }[];
    hits = result.map((h) => h.key);
  }
</script>

<div class="settings">
  <nav aria-label={t("settings.sections")}>
    <input
      type="search"
      class="search"
      aria-label={t("settings.search.label")}
      placeholder={t("settings.search.placeholder")}
      bind:value={query}
      oninput={search}
    />
    {#each sections as s (s)}
      <button
        type="button"
        aria-current={hits === null && s === section ? "page" : undefined}
        onclick={() => ((section = s), (hits = null), (query = ""))}
      >
        <Icon name={icons[s] ?? "gear"} size={14} />
        {t(`settings.section.${s}`)}
      </button>
    {/each}
  </nav>
  <main>
    <header>
      <h1>{hits === null ? t(`settings.section.${section}`) : t("settings.title")}</h1>
      <label class="changed"
        ><input type="checkbox" bind:checked={changedOnly} /> {t("settings.changedOnly")}</label
      >
    </header>
    {#if rows.length === 0}
      <p class="empty">{t("settings.noResults")}</p>
    {:else}
      <div class="group">
        {#each rows as entry (entry.key)}
          <SettingRow
            {entry}
            value={values[entry.key]}
            client={store.client}
            controlledBy={controller(entry.key)}
          />
        {/each}
      </div>
    {/if}
    {#if hits === null && section === "privacy"}
      <BlockerSection {store} />
    {/if}
    {#if hits === null && section === "passwords"}
      <PasswordsSection {store} />
      <PasswordSources {store} />
    {/if}
    {#if hits === null && section === "ai"}
      <AiConnections {store} />
      <AiModels {store} />
      <MemorySection {store} />
    {/if}
    {#if hits === null && section === "developers"}
      <ControlClients {store} />
      <AgentSetup {store} />
    {/if}
    {#if hits === null && section === "extensions"}
      <ExtensionsSection {store} />
    {/if}
    {#if hits === null && section === "profiles"}
      <ProfilesSection client={store.client} />
    {/if}
    {#if hits === null && section === "shortcuts"}
      <button
        type="button"
        onclick={() =>
          store.client.call({ type: "settings.reset", section: "shortcuts" }).catch(() => {})}
        >{t("settings.shortcut.resetAll")}</button
      >
    {/if}
    {#if hits === null && section === "general"}
      <DefaultBrowser client={store.client} />
      <ImportSheet client={store.client} />
    {/if}
    {#if hits === null && section === "privacy"}
      <ClearData client={store.client} />
    {/if}
    {#if hits === null && section === "siteSettings"}
      <SiteExceptions client={store.client} />
    {/if}
  </main>
  <Notices {store} />
</div>

<style>
  .settings {
    display: grid;
    grid-template-columns: 210px 1fr;
    height: 100vh;
    background: var(--chrome);
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: var(--space-5) var(--space-4);
    overflow-y: auto;
    color: var(--text-on-chrome);
  }
  .search {
    margin-bottom: var(--space-5);
    border-color: transparent !important;
    background: var(--fill) !important;
  }
  nav button {
    justify-content: flex-start;
    gap: var(--space-4);
    min-height: 28px;
    padding: 0 var(--space-4);
    border: 0;
    background: transparent;
    font-weight: var(--weight-regular);
    color: inherit;
  }
  nav button:hover {
    background: var(--fill-hover) !important;
  }
  nav button[aria-current="page"] {
    background: var(--chrome-active) !important;
    box-shadow: 0 0 0 1px var(--line);
  }
  main {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-8) var(--space-9) var(--space-10);
    background: var(--surface);
    color: var(--text);
  }
  main > * {
    width: 100%;
    max-width: 620px;
    margin-inline: auto;
  }
  header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }
  h1 {
    font-size: var(--text-heading);
  }
  .changed {
    font-size: var(--text-small);
    color: var(--text-secondary);
  }
  .group,
  main > :global(section),
  main > :global(div:not(.group)) {
    border: 1px solid var(--line);
    border-radius: var(--radius-card);
    background: var(--surface-raised);
  }
  main > :global(section),
  main > :global(div:not(.group)) {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-5) var(--space-6);
  }
  main :global(section h3) {
    font-size: var(--text-body);
  }
  main :global(section li) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) 0;
    border-top: 1px solid var(--line);
  }
  .empty {
    color: var(--text-secondary);
  }
</style>
