<script lang="ts">
  // The welcome sheet (M2-T17): import from another browser, pick a search
  // engine (none preselected) and whether to show suggestions, and offer to
  // make Mado the default browser. Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";
  import ImportSheet from "./ImportSheet.svelte";

  type Engine = { id: string; name: string; active: boolean };
  let { store }: { store: CoreStore } = $props();

  let step = $state<"import" | "search" | "passwords" | "browser">("import");
  let secret = $state("");
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

  async function toPasswords() {
    if (engine) await set("search.defaultEngine", engine);
    await set("search.suggestions", suggestions ? "on" : "off");
    step = (store.state?.vault?.status ?? "none") === "none" ? "passwords" : "browser";
    if (step === "browser") await toBrowser();
  }

  // The password manager step (11, roadmap M4-T12): Mado Passwords now, or
  // 1Password and Bitwarden in Settings › Passwords.
  async function useMado() {
    const r = (await store.client.call({ type: "vault.setup" }).catch(() => null)) as {
      recoverySecret: string;
    } | null;
    secret = r?.recoverySecret ?? "";
  }

  async function toBrowser() {
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

<div class="first-run" role="dialog" aria-labelledby="first-run-title">
  <div class="sheet card floating">
    <h1 id="first-run-title">{t("firstRun.title")}</h1>
    {#if step === "import"}
      <ImportSheet client={store.client} />
      <button type="button" onclick={toSearch}>{t("firstRun.next")}</button>
    {:else if step === "search"}
      <h2>{t("firstRun.search")}</h2>
      {#each engines as e (e.id)}
        <label><input type="radio" name="engine" value={e.id} bind:group={engine} /> {e.name}</label
        >
      {/each}
      <label><input type="checkbox" bind:checked={suggestions} /> {t("firstRun.suggestions")}</label
      >
      <p class="help">{t("firstRun.suggestionsHelp")}</p>
      <button type="button" disabled={!engine} onclick={toPasswords}>{t("firstRun.next")}</button>
    {:else if step === "passwords"}
      <h2>{t("firstRun.passwords")}</h2>
      {#if secret}
        <p>{t("passwords.secret")}</p>
        <pre>{secret}</pre>
        <button type="button" onclick={toBrowser}>{t("passwords.secret.saved")}</button>
      {:else}
        <button type="button" onclick={useMado}>{t("firstRun.passwords.mado")}</button>
        <button
          type="button"
          onclick={() =>
            store.client
              .call({ type: "app.openSettings", section: "passwords" })
              .catch(() => {})
              .then(toBrowser)}>{t("firstRun.passwords.other")}</button
        >
        <button type="button" onclick={toBrowser}>{t("firstRun.notNow")}</button>
      {/if}
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
</div>

<style>
  .first-run {
    position: fixed;
    inset: 0;
    z-index: 10;
    display: grid;
    place-items: center;
    padding: var(--space-9);
    background: var(--chrome);
    color: var(--text);
  }
  .sheet {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-5);
    width: min(600px, 100%);
    max-height: 100%;
    overflow: auto;
    padding: var(--space-9);
    border-radius: var(--radius-sheet);
  }
  .sheet :global(h1) {
    font-size: var(--text-display);
    font-weight: var(--weight-display);
  }
  .sheet :global(label) {
    margin-right: var(--space-5);
  }
  .sheet :global(section),
  .sheet :global(div) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-4);
  }
  .sheet > :global(button:last-of-type) {
    align-self: flex-end;
  }
  pre {
    padding: var(--space-5);
    border-radius: var(--radius-control);
    background: var(--surface-sunken);
    font-family: var(--font-mono);
    -webkit-user-select: all;
    user-select: all;
  }
  .help {
    color: var(--text-secondary);
    font-size: var(--text-small);
  }
</style>
