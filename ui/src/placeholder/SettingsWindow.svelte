<script lang="ts">
  // The settings window (label `settings`, Origin::Settings). Sections and
  // rows come from the registry; search asks the core. Placeholder until M6.
  import { type SettingEntry, entries, sections, visible } from "../core/settings-index";
  import type { CoreStore } from "../core/store.svelte";
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

  let { store }: { store: CoreStore } = $props();

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
    {#each sections as s (s)}
      <button
        type="button"
        aria-current={hits === null && s === section ? "page" : undefined}
        onclick={() => ((section = s), (hits = null), (query = ""))}
      >
        {t(`settings.section.${s}`)}
      </button>
    {/each}
  </nav>
  <main>
    <h1>{t("settings.title")}</h1>
    <input
      type="search"
      aria-label={t("settings.search.label")}
      placeholder={t("settings.search.placeholder")}
      bind:value={query}
      oninput={search}
    />
    <label><input type="checkbox" bind:checked={changedOnly} /> {t("settings.changedOnly")}</label>
    {#if rows.length === 0}
      <p>{t("settings.noResults")}</p>
    {/if}
    {#each rows as entry (entry.key)}
      <SettingRow
        {entry}
        value={values[entry.key]}
        client={store.client}
        controlledBy={controller(entry.key)}
      />
    {/each}
    {#if hits === null && section === "privacy"}
      <BlockerSection {store} />
    {/if}
    {#if hits === null && section === "passwords"}
      <PasswordsSection {store} />
      <PasswordSources {store} />
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
