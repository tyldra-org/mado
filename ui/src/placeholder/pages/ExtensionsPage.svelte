<script lang="ts">
  // mado://extensions (spec 09 §11): the installed extensions. Managing them
  // needs Mado's own window, so the page opens Settings › Extensions.
  // Placeholder until the design pass (M6).
  import type { CoreStore } from "../../core/store.svelte";
  import { t } from "../../i18n";

  let { store }: { store: CoreStore } = $props();
  const installed = $derived(store.state?.extensions?.installed ?? []);
</script>

<main>
  <h1>{t("internal.extensions.title")}</h1>
  <ul>
    {#each installed as e (e.id)}
      <li>
        {e.name}
        {e.version}
        {#if !e.enabled}({t("extensions.off")}){/if}
        {#if e.hasOptions}
          <button
            type="button"
            onclick={() =>
              store.client.call({ type: "extension.openOptions", id: e.id }).catch(() => {})}
            >{t("extensions.options")}</button
          >
        {/if}
      </li>
    {/each}
  </ul>
  <button
    type="button"
    onclick={() =>
      store.client.call({ type: "app.openSettings", section: "extensions" }).catch(() => {})}
    >{t("extensions.manage")}</button
  >
</main>
