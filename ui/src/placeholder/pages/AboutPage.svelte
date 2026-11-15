<script lang="ts">
  // mado://about (spec 15 §5): this build, the web engine under it, and
  // updates. Placeholder until the design pass (M6).
  import type { CoreStore } from "../../core/store.svelte";
  import { t } from "../../i18n";

  let { store }: { store: CoreStore } = $props();
  const update = $derived(store.state?.update);
  const about = $derived(store.state?.about);
</script>

<main>
  <h1>{t("internal.about.title")}</h1>
  {#if about}
    <p>{t("internal.about.version", { version: about.version })}</p>
    <p>{t("internal.about.engine", { version: about.engineVersion || "?" })}</p>
    {#if about.engineOutdated}
      <div role="alert">
        <p>{t("internal.about.engineOutdated")}</p>
        <button
          type="button"
          onclick={() =>
            store.client
              .call({ type: "app.openSystemSettings", pane: "softwareUpdate" })
              .catch(() => {})}>{t("internal.about.softwareUpdate")}</button
        >
      </div>
    {/if}
  {/if}
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
        disabled={update.status === "checking" || update.status === "downloading"}
        onclick={() => store.client.call({ type: "update.check" }).catch(() => {})}
        >{t("internal.about.check")}</button
      >
    {/if}
  {/if}
  <p><a href="mado://credits">{t("internal.credits.title")}</a></p>
</main>
