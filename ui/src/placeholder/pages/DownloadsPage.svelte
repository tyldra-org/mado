<script lang="ts">
  // mado://downloads: every download, from the mirrored `downloads` state.
  import type { CoreStore } from "../../core/store.svelte";
  import type { Command } from "../../generated/Command";
  import type { DownloadState } from "../../generated/DownloadState";
  import { t } from "../../i18n";

  let { store }: { store: CoreStore } = $props();
  const list = $derived(
    Object.values((store.state?.downloads ?? {}) as Record<string, DownloadState>).sort(
      (a, b) => b.startedAt - a.startedAt,
    ),
  );

  function act(type: string, downloadId: string) {
    void store.client.call({ type, downloadId } as Command).catch(() => {});
  }
</script>

<main>
  <h1>{t("internal.downloads.title")}</h1>
  {#if list.length === 0}<p>{t("internal.downloads.empty")}</p>{/if}
  <ul>
    {#each list as d (d.id)}
      <li>
        <span>{d.filename}</span>
        <span>{t(`internal.downloads.state.${d.state}`)}</span>
        {#if d.state === "in_progress"}
          <button type="button" onclick={() => act("download.pause", d.id)}
            >{t("internal.downloads.pause")}</button
          >
          <button type="button" onclick={() => act("download.cancel", d.id)}
            >{t("internal.downloads.cancel")}</button
          >
        {:else if d.state === "paused" || d.state === "failed"}
          <button type="button" onclick={() => act("download.resume", d.id)}
            >{t("internal.downloads.resume")}</button
          >
        {:else if d.state === "complete"}
          <button type="button" onclick={() => act("download.open", d.id)}
            >{t("internal.downloads.open")}</button
          >
          <button type="button" onclick={() => act("download.reveal", d.id)}
            >{t("internal.downloads.reveal")}</button
          >
        {/if}
        {#if d.state !== "in_progress"}
          <button type="button" onclick={() => act("download.remove", d.id)}
            >{t("internal.downloads.remove")}</button
          >
        {/if}
      </li>
    {/each}
  </ul>
  {#if list.length}
    <button
      type="button"
      onclick={() => store.client.call({ type: "download.clearList" }).catch(() => {})}
      >{t("internal.downloads.clear")}</button
    >
  {/if}
</main>
