<script lang="ts">
  // The settings window (label `settings`, Origin::Settings). Sections and
  // rows come from the registry; search asks the core. Placeholder until M6.
  import { type SettingEntry, entries, sections, visible } from "../core/settings-index";
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";
  import Notices from "./Notices.svelte";
  import SettingRow from "./SettingRow.svelte";

  let { store }: { store: CoreStore } = $props();

  let query = $state("");
  let changedOnly = $state(false);
  let section = $state(sections[0] ?? "general");
  /** Keys from settings.search, best first; null while not searching. */
  let hits: string[] | null = $state(null);

  const values = $derived(store.state?.settings.values ?? {});
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
      <SettingRow {entry} value={values[entry.key]} client={store.client} />
    {/each}
  </main>
  <Notices {store} />
</div>
