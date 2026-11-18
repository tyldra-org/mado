<script lang="ts">
  // mado://history: visits by day, search, delete.
  import type { CoreStore } from "../../core/store.svelte";
  import { t } from "../../i18n";
  import { openUrl } from "./open";

  type Visit = { visitId: number; url: string; title: string; visitedAt: number };
  type Day = { day: string; visits: Visit[] };
  let { store }: { store: CoreStore } = $props();
  let text = $state("");
  let days = $state<Day[]>([]);

  async function load() {
    const r = (await store.client
      .call({
        type: "history.query",
        text: text.trim() || null,
        host: null,
        from: null,
        to: null,
        limit: 100,
        cursor: null,
        utcOffsetMinutes: -new Date().getTimezoneOffset(),
      })
      .catch(() => ({ days: [] }))) as { days: Day[] };
    days = r.days;
  }

  async function remove(id: number) {
    await store.client
      .call({ type: "history.delete", visitIds: [id], urlIds: [], host: null, range: null })
      .catch(() => {});
    await load();
  }

  $effect(() => {
    void load();
  });
</script>

<main>
  <h1>{t("internal.history.title")}</h1>
  <input
    type="search"
    placeholder={t("internal.history.search")}
    aria-label={t("internal.history.search")}
    bind:value={text}
    oninput={load}
  />
  {#if days.length === 0}<p>{t("internal.history.empty")}</p>{/if}
  {#each days as d (d.day)}
    <h2>{d.day}</h2>
    <ul>
      {#each d.visits as v (v.visitId)}
        <li>
          <button type="button" onclick={() => openUrl(store, v.url)}>{v.title || v.url}</button>
          <button
            type="button"
            aria-label={t("internal.history.remove", { title: v.title || v.url })}
            onclick={() => remove(v.visitId)}>×</button
          >
        </li>
      {/each}
    </ul>
  {/each}
</main>
