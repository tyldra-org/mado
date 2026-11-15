<script lang="ts">
  // The site-info panel next to the address bar (spec 07 §5, 08): the
  // connection, what the site may use, and what it is using now.
  // Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  type Row = {
    origin: string;
    key: string;
    value: "allow" | "block" | "ask";
    embedder: string;
    source: string;
  };

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  const tab = $derived.by(() => {
    const id = store.state?.windows[windowId]?.activeTabId;
    return id ? store.state?.tabs[id] : undefined;
  });
  const origin = $derived.by(() => {
    try {
      const u = new URL(tab?.url ?? "");
      return u.protocol === "http:" || u.protocol === "https:" ? u.origin : null;
    } catch {
      return null;
    }
  });

  let open = $state(false);
  let rows = $state<Row[]>([]);

  async function load() {
    if (!origin) return;
    rows = (
      (await store.client.call({ type: "permission.list", origin }).catch(() => [])) as Row[]
    ).filter((r) => r.embedder === "");
  }

  async function set(key: string, value: "allow" | "block" | "ask") {
    if (!origin) return;
    await store.client
      .call({ type: "permission.set", origin, key, value, embedder: null })
      .catch(() => {});
    await load();
  }

  async function reset() {
    if (!origin) return;
    await store.client.call({ type: "permission.reset", origin, key: null }).catch(() => {});
    await load();
  }

  let cleared = $state(false);
  async function clearData() {
    if (!origin) return;
    await store.client.call({ type: "site.clearData", origin }).catch(() => {});
    cleared = true;
  }

  const capturing = $derived(
    (["camera", "microphone", "screen", "location"] as const).filter((k) => tab?.capture?.[k]),
  );
</script>

{#if origin}
  <button
    type="button"
    aria-expanded={open}
    onclick={() => ((open = !open), (cleared = false), open && load())}
    >{t("siteInfo.button")}</button
  >
{/if}
{#if open && origin && tab}
  <div role="dialog" aria-labelledby="site-info-title">
    <h2 id="site-info-title">{new URL(origin).host}</h2>
    <p>{t(`address.security.${tab.security?.level ?? "internal"}`)}</p>
    {#each capturing as kind (kind)}
      <p>
        {t("siteInfo.using", { what: t(`permission.${kind}`) })}
        <button
          type="button"
          onclick={() =>
            store.client.call({ type: "site.stopCapture", tabId: tab.id, kind }).catch(() => {})}
          >{t("siteInfo.stop")}</button
        >
      </p>
    {/each}
    {#if rows.length === 0}<p>{t("siteInfo.noPermissions")}</p>{/if}
    <ul>
      {#each rows as r (r.key)}
        <li>
          <label>
            {t(`permission.${r.key}`)}
            <select
              value={r.value}
              onchange={(e) => set(r.key, e.currentTarget.value as Row["value"])}
            >
              <option value="allow">{t("siteValue.allow")}</option>
              <option value="block">{t("siteValue.block")}</option>
              <option value="ask">{t("siteValue.ask")}</option>
            </select>
          </label>
          {#if r.source === "default"}<span>{t("siteInfo.blockedAutomatically")}</span>{/if}
        </li>
      {/each}
    </ul>
    {#if rows.length}<button type="button" onclick={reset}>{t("siteInfo.reset")}</button>{/if}
    <button type="button" onclick={clearData}>{t("siteInfo.clearData")}</button>
    {#if cleared}<p role="status">{t("siteInfo.dataCleared")}</p>{/if}
  </div>
{/if}
