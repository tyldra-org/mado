<script lang="ts">
  // mado://about: the update status and button (M2-T16 completes it).
  import type { CoreStore } from "../../core/store.svelte";
  import { t } from "../../i18n";

  let { store }: { store: CoreStore } = $props();
  const update = $derived(store.state?.update);
</script>

<main>
  <h1>{t("internal.about.title")}</h1>
  {#if update}
    <p role="status">
      {t(`internal.about.update.${update.status}`, { version: update.version ?? "" })}
    </p>
    {#if update.status === "ready"}
      <button
        type="button"
        onclick={() => store.client.call({ type: "update.relaunch" }).catch(() => {})}
        >{t("internal.about.relaunch")}</button
      >
    {:else}
      <button
        type="button"
        onclick={() => store.client.call({ type: "update.check" }).catch(() => {})}
        >{t("internal.about.check")}</button
      >
    {/if}
  {/if}
  <p><a href="mado://credits">{t("internal.credits.title")}</a></p>
</main>
