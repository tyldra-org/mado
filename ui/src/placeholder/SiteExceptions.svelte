<script lang="ts">
  // Site settings' exceptions (spec 07): every site's own values, changed or
  // forgotten here. The defaults are the registry rows above. Placeholder
  // until M6.
  import type { Client } from "../core/client";
  import { t } from "../i18n";

  type Row = {
    origin: string;
    key: string;
    value: "allow" | "block" | "ask";
    embedder: string;
    source: string;
  };

  let { client }: { client: Client } = $props();
  let rows = $state<Row[]>([]);

  async function load() {
    rows = (await client.call({ type: "permission.list", origin: null }).catch(() => [])) as Row[];
  }
  $effect(() => {
    void load();
  });

  const origins = $derived([...new Set(rows.map((r) => r.origin))].sort());

  async function set(r: Row, value: Row["value"]) {
    await client
      .call({
        type: "permission.set",
        origin: r.origin,
        key: r.key,
        value,
        embedder: r.embedder || null,
      })
      .catch(() => {});
    await load();
  }
  async function forget(origin: string) {
    await client.call({ type: "permission.reset", origin, key: null }).catch(() => {});
    await load();
  }
</script>

<section aria-labelledby="exceptions-title">
  <h2 id="exceptions-title">{t("siteSettings.exceptions")}</h2>
  {#if origins.length === 0}<p>{t("siteSettings.none")}</p>{/if}
  {#each origins as origin (origin)}
    <h3>{origin}</h3>
    <ul>
      {#each rows.filter((r) => r.origin === origin) as r (r.key + r.embedder)}
        <li>
          <label>
            {t(`permission.${r.key}`)}
            <select value={r.value} onchange={(e) => set(r, e.currentTarget.value as Row["value"])}>
              <option value="allow">{t("siteValue.allow")}</option>
              <option value="block">{t("siteValue.block")}</option>
              <option value="ask">{t("siteValue.ask")}</option>
            </select>
          </label>
          {#if r.source === "default"}<span>{t("siteInfo.blockedAutomatically")}</span>{/if}
          {#if r.source === "auto_revoked"}<span>{t("siteSettings.revoked")}</span>{/if}
        </li>
      {/each}
    </ul>
    <button type="button" onclick={() => forget(origin)}
      >{t("siteSettings.forget", { origin })}</button
    >
  {/each}
</section>
