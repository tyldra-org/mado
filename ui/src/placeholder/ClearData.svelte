<script lang="ts">
  // Clear browsing data (spec 08 §7), in the Privacy section of Settings.
  // Placeholder until the design pass (M6).
  import type { Client } from "../core/client";
  import type { ClearRange } from "../generated/ClearRange";
  import type { DataType } from "../generated/DataType";
  import { t } from "../i18n";

  let { client }: { client: Client } = $props();

  const RANGES: ClearRange[] = ["hour", "day", "week", "month", "all"];
  const TYPES: DataType[] = [
    "history",
    "downloadsList",
    "cookies",
    "cache",
    "siteSettings",
    "formHistory",
    "blockerStats",
    "aiChats",
  ];

  let range = $state<ClearRange>("hour");
  let chosen = $state<DataType[]>(["history", "cookies", "cache"]);
  let status = $state<"idle" | "clearing" | "done" | "failed">("idle");

  async function clear() {
    status = "clearing";
    try {
      await client.call({ type: "data.clear", range, types: chosen });
      status = "done";
    } catch {
      status = "failed";
    }
  }
</script>

<section aria-labelledby="clear-data-title">
  <h2 id="clear-data-title">{t("clearData.title")}</h2>
  <label>
    {t("clearData.range")}
    <select bind:value={range}>
      {#each RANGES as r (r)}<option value={r}>{t(`clearData.range.${r}`)}</option>{/each}
    </select>
  </label>
  {#each TYPES as type (type)}
    <label
      ><input type="checkbox" value={type} bind:group={chosen} />
      {t(`clearData.type.${type}`)}</label
    >
  {/each}
  <p class="help">{t("clearData.passwords")}</p>
  <button type="button" disabled={chosen.length === 0 || status === "clearing"} onclick={clear}
    >{t("clearData.clear")}</button
  >
  {#if status === "done"}<p role="status">{t("clearData.done")}</p>{/if}
  {#if status === "failed"}<p role="alert">{t("clearData.failed")}</p>{/if}
</section>
