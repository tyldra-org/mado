<script lang="ts">
  // Import from another browser (spec 12 §5), in Settings › General.
  // Placeholder until the design pass (M6).
  import type { Client } from "../core/client";
  import { CoreError } from "../core/client";
  import type { ImportBrowser } from "../generated/ImportBrowser";
  import type { ImportType } from "../generated/ImportType";
  import { t } from "../i18n";

  type Found = { browser: ImportBrowser; profile: string; name: string };
  type Counts = {
    bookmarks: number;
    visits: number;
    engines: number;
    tabs: number;
    extensions: number;
  };

  let { client }: { client: Client } = $props();

  const TYPES: ImportType[] = ["bookmarks", "history", "searchEngines", "tabs", "extensions"];
  let found = $state<Found[] | null>(null);
  let chosen = $state(0);
  let types = $state<ImportType[]>(["bookmarks", "history", "searchEngines"]);
  let allHistory = $state(false);
  let status = $state<"idle" | "running" | "done" | "fullDiskAccess" | "failed">("idle");
  let counts = $state<Counts | null>(null);

  async function detect() {
    found = (await client.call({ type: "import.detect" }).catch(() => [])) as Found[];
  }

  async function run() {
    const source = found?.[chosen];
    if (!source) return;
    status = "running";
    try {
      counts = (await client.call({
        type: "import.run",
        browser: source.browser,
        profile: source.profile,
        types,
        allHistory,
      })) as Counts;
      status = "done";
    } catch (e) {
      status =
        e instanceof CoreError && e.error.messageKey === "error.import.fullDiskAccess"
          ? "fullDiskAccess"
          : "failed";
    }
  }

  // After granting Full Disk Access in System Settings, coming back to Mado
  // tries again (12 §5.3).
  $effect(() => {
    const retry = () => {
      if (status === "fullDiskAccess") void run();
    };
    window.addEventListener("focus", retry);
    return () => window.removeEventListener("focus", retry);
  });

  $effect(() => {
    void detect();
  });
</script>

<section aria-labelledby="import-title">
  <h2 id="import-title">{t("import.title")}</h2>
  {#if found !== null && found.length === 0}
    <p>{t("import.none")}</p>
  {:else if found !== null}
    <label>
      {t("import.from")}
      <select bind:value={chosen}>
        {#each found as f, i (i)}
          <option value={i}
            >{f.browser === "safari"
              ? "Safari"
              : `${t(`import.browser.${f.browser}`)} — ${f.name}`}</option
          >
        {/each}
      </select>
    </label>
    {#each TYPES as type (type)}
      <label
        ><input type="checkbox" value={type} bind:group={types} /> {t(`import.type.${type}`)}</label
      >
    {/each}
    {#if types.includes("history")}
      <label><input type="checkbox" bind:checked={allHistory} /> {t("import.allHistory")}</label>
    {/if}
    <button type="button" disabled={types.length === 0 || status === "running"} onclick={run}
      >{t("import.run")}</button
    >
  {/if}
  {#if status === "done" && counts}
    <p role="status">
      {t("import.done", { bookmarks: counts.bookmarks, visits: counts.visits, tabs: counts.tabs })}
    </p>
  {/if}
  {#if status === "fullDiskAccess"}
    <div role="alert">
      <p>{t("import.fullDiskAccess")}</p>
      <button
        type="button"
        onclick={() =>
          client.call({ type: "app.openSystemSettings", pane: "fullDiskAccess" }).catch(() => {})}
        >{t("import.openSettings")}</button
      >
    </div>
  {/if}
  {#if status === "failed"}<p role="alert">{t("import.failed")}</p>{/if}
</section>
