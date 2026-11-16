<script lang="ts">
  // Settings › Privacy › Blocker (spec 10 §8, 08 §9): the filter lists, custom
  // filters, updates and the privacy report. Placeholder until M6.
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();

  type List = {
    id: string;
    kind: string;
    enabled: boolean;
    filters: number;
    updatedAt: number | null;
  };
  type Report = {
    ads: number;
    trackers: number;
    sites: { host: string; ads: number; trackers: number }[];
    trackerDomains: { domain: string; count: number }[];
  };

  const blocker = $derived(store.state?.blocker);
  let lists: List[] = $state([]);
  let custom = $state("");
  let days: 7 | 30 = $state(7);
  let report: Report | null = $state(null);

  const call = (cmd: Parameters<CoreStore["client"]["call"]>[0]) => store.client.call(cmd);

  async function load() {
    lists = ((await call({ type: "blocker.availableLists" }).catch(() => [])) as List[]) ?? [];
    custom = String((await call({ type: "blocker.customFilters" }).catch(() => "")) ?? "");
    report = (await call({ type: "site.privacyReport", days }).catch(() => null)) as Report | null;
  }

  $effect(() => {
    void blocker?.listsUpdatedAt;
    void days;
    void load();
  });

  async function toggle(id: string, on: boolean) {
    const ids = lists.filter((l) => (l.id === id ? on : l.enabled)).map((l) => l.id);
    await call({ type: "blocker.setLists", ids }).catch(() => {});
    await load();
  }
</script>

<section data-testid="blocker-section">
  <h2>{t("blocker.title")}</h2>
  {#if blocker}
    <p>
      {t("blocker.filters", { count: blocker.filters })}
      {#if blocker.bundled}({t("blocker.bundled")}){/if}
      {#if blocker.listsUpdatedAt}{t("blocker.updated", {
          date: new Date(blocker.listsUpdatedAt).toLocaleString(),
        })}{/if}
    </p>
    <button
      type="button"
      disabled={blocker.updating}
      onclick={() => call({ type: "blocker.updateNow" }).catch(() => {})}
      >{blocker.updating ? t("blocker.updating") : t("blocker.updateNow")}</button
    >
  {/if}
  <h3>{t("blocker.lists")}</h3>
  <ul>
    {#each lists as l (l.id)}
      <li>
        <label
          ><input
            type="checkbox"
            checked={l.enabled}
            onchange={(e) => toggle(l.id, e.currentTarget.checked)}
          />
          {l.id} ({t(`blocker.kind.${l.kind}`)}){l.filters ? ` · ${l.filters}` : ""}</label
        >
      </li>
    {/each}
  </ul>
  <h3>{t("blocker.custom")}</h3>
  <textarea bind:value={custom} rows="6" aria-label={t("blocker.custom")}></textarea>
  <button
    type="button"
    onclick={() => call({ type: "blocker.setCustomFilters", text: custom }).catch(() => {})}
    >{t("blocker.save")}</button
  >
  {#if blocker && blocker.offHosts.length > 0}
    <h3>{t("blocker.offSites")}</h3>
    <ul>
      {#each blocker.offHosts as host (host)}
        <li>
          {host}
          <button
            type="button"
            onclick={() => call({ type: "blocker.setSite", host, enabled: true }).catch(() => {})}
            >{t("blocker.turnOn")}</button
          >
        </li>
      {/each}
    </ul>
  {/if}
  <h3>{t("blocker.report")}</h3>
  <select bind:value={days} aria-label={t("blocker.report")}>
    <option value={7}>{t("blocker.report.week")}</option>
    <option value={30}>{t("blocker.report.month")}</option>
  </select>
  {#if report}
    <p data-testid="privacy-report">
      {t("blocker.report.totals", { ads: report.ads, trackers: report.trackers })}
    </p>
    <ol>
      {#each report.sites as s (s.host)}<li>{s.host}: {s.ads + s.trackers}</li>{/each}
    </ol>
    <ol>
      {#each report.trackerDomains as d (d.domain)}<li>{d.domain}: {d.count}</li>{/each}
    </ol>
  {/if}
</section>
