<script lang="ts">
  // The toolbar's downloads button and bubble (spec 12 §4.2 step 6, §4.3).
  // Placeholder until the design pass (M6); `mado://downloads` lists the
  // same downloads with the internal pages (M2-T14).
  import type { CoreStore } from "../core/store.svelte";
  import type { DownloadState } from "../generated/DownloadState";
  import Icon from "../design/Icon.svelte";
  import { portal } from "../design/portal";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();

  const downloads = $derived(
    Object.values(store.state?.downloads ?? {})
      .filter((d): d is DownloadState => !!d)
      .sort((a, b) => b.startedAt - a.startedAt),
  );

  let open = $state(false);
  let seen = new Set<string>();
  let first = true;

  // The bubble opens when a download starts, not for the list loaded at
  // launch.
  $effect(() => {
    const ids = downloads.map((d) => d.id);
    if (!first && ids.some((id) => !seen.has(id))) open = true;
    seen = new Set(ids);
    first = store.state === null;
  });

  const size = new Intl.NumberFormat("en", {
    notation: "compact",
    style: "unit",
    unit: "byte",
    unitDisplay: "narrow",
  });

  const host = (url: string) => {
    try {
      return new URL(url).host;
    } catch {
      return url;
    }
  };

  type Action =
    | "download.pause"
    | "download.resume"
    | "download.cancel"
    | "download.keep"
    | "download.discard"
    | "download.open"
    | "download.reveal"
    | "download.remove";

  const act = (type: Action, downloadId: string) =>
    void store.client.call({ type, downloadId }).catch(() => {});

  const actions = (d: DownloadState): [Action, string][] => {
    switch (d.state) {
      case "in_progress":
        return [
          ["download.pause", "downloads.pause"],
          ["download.cancel", "downloads.cancel"],
        ];
      case "paused":
        return [
          ["download.resume", "downloads.resume"],
          ["download.cancel", "downloads.cancel"],
        ];
      case "failed":
        return d.error === "interrupted"
          ? [
              ["download.resume", "downloads.resume"],
              ["download.remove", "downloads.remove"],
            ]
          : [["download.remove", "downloads.remove"]];
      case "dangerous":
      case "blocked":
        return [
          ["download.keep", "downloads.keep"],
          ["download.discard", "downloads.discard"],
        ];
      case "complete":
        return [
          ["download.open", "downloads.open"],
          ["download.reveal", "downloads.reveal"],
          ["download.remove", "downloads.remove"],
        ];
      default:
        return [["download.remove", "downloads.remove"]];
    }
  };
</script>

{#if downloads.length}
  <button
    type="button"
    class="icon"
    aria-expanded={open}
    aria-label={t("downloads.show", { count: downloads.length })}
    title={t("downloads.show", { count: downloads.length })}
    onclick={() => (open = !open)}><Icon name="download" /></button
  >
{/if}

{#if open}
  <div
    class="popover card floating"
    role="dialog"
    aria-labelledby="downloads-title"
    {@attach portal("popovers")}
  >
    <h2 id="downloads-title">{t("downloads.title")}</h2>
    <ul>
      {#each downloads as d (d.id)}
        <li>
          <strong>{d.filename}</strong>
          <span>
            {d.state === "dangerous"
              ? t("downloads.state.dangerous", { host: host(d.url) })
              : t(`downloads.state.${d.state}`)}
          </span>
          {#if d.state === "in_progress" || d.state === "paused"}
            <progress max={d.total ?? undefined} value={d.received}></progress>
            <span>
              {d.total
                ? t("downloads.progress", {
                    received: size.format(d.received),
                    total: size.format(d.total),
                  })
                : t("downloads.progressUnknown", { received: size.format(d.received) })}
            </span>
          {/if}
          {#each actions(d) as [type, label] (type)}
            <button type="button" onclick={() => act(type, d.id)}>{t(label)}</button>
          {/each}
        </li>
      {/each}
    </ul>
    <button
      type="button"
      onclick={() => void store.client.call({ type: "download.clearList" }).catch(() => {})}
      >{t("downloads.clear")}</button
    >
    <button type="button" onclick={() => (open = false)}>{t("downloads.close")}</button>
  </div>
{/if}
