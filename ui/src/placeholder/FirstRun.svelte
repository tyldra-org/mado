<script lang="ts">
  // The welcome sheet (M2-T17): import from another browser, pick a search
  // engine (none preselected) and whether to show suggestions, and offer to
  // make Mado the default browser. Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";
  import ImportSheet from "./ImportSheet.svelte";

  type Engine = { id: string; name: string; active: boolean };
  let { store }: { store: CoreStore } = $props();

  let step = $state<"import" | "search" | "browser">("import");
  let engines = $state<Engine[]>([]);
  let engine = $state("");
  let suggestions = $state(false);
  let isDefault = $state(false);

  const set = (key: string, value: unknown) =>
    store.client.call({ type: "settings.set", key, value, token: null }).catch(() => {});

  async function toSearch() {
    engines = (
      (await store.client.call({ type: "nav.searchEngines" }).catch(() => [])) as Engine[]
    ).filter((e) => e.active);
    step = "search";
  }

  async function toBrowser() {
    if (engine) await set("search.defaultEngine", engine);
    await set("search.suggestions", suggestions ? "on" : "off");
    const d = (await store.client.call({ type: "app.defaultBrowser" }).catch(() => null)) as {
      isDefault: boolean;
    } | null;
    isDefault = d?.isDefault ?? false;
    step = "browser";
  }

  async function makeDefault() {
    await store.client.call({ type: "app.setDefaultBrowser" }).catch(() => {});
    await finish();
  }

  const finish = () => store.client.call({ type: "app.finishFirstRun" }).catch(() => {});
</script>

<div role="dialog" aria-labelledby="first-run-title">
  <h1 id="first-run-title">{t("firstRun.title")}</h1>
  {#if step === "import"}
    <ImportSheet client={store.client} />
    <button type="button" onclick={toSearch}>{t("firstRun.next")}</button>
  {:else if step === "search"}
    <h2>{t("firstRun.search")}</h2>
    {#each engines as e (e.id)}
      <label><input type="radio" name="engine" value={e.id} bind:group={engine} /> {e.name}</label>
    {/each}
    <label><input type="checkbox" bind:checked={suggestions} /> {t("firstRun.suggestions")}</label>
    <p class="help">{t("firstRun.suggestionsHelp")}</p>
    <button type="button" disabled={!engine} onclick={toBrowser}>{t("firstRun.next")}</button>
  {:else}
    <h2>{t("firstRun.browser")}</h2>
    {#if isDefault}
      <p>{t("firstRun.alreadyDefault")}</p>
    {:else}
      <button type="button" onclick={makeDefault}>{t("firstRun.makeDefault")}</button>
    {/if}
    <button type="button" onclick={finish}
      >{t(isDefault ? "firstRun.done" : "firstRun.notNow")}</button
    >
  {/if}
</div>
