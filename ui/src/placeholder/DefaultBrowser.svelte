<script lang="ts">
  // Settings › General: the default browser offer (decisions: offered at
  // first run and here, no reminders).
  import type { Client } from "../core/client";
  import { t } from "../i18n";

  let { client }: { client: Client } = $props();
  let isDefault = $state<boolean | null>(null);

  async function load() {
    const d = (await client.call({ type: "app.defaultBrowser" }).catch(() => null)) as {
      isDefault: boolean;
    } | null;
    isDefault = d?.isDefault ?? null;
  }

  $effect(() => {
    void load();
  });
</script>

<section aria-labelledby="default-browser-title">
  <h2 id="default-browser-title">{t("firstRun.browser")}</h2>
  {#if isDefault}
    <p>{t("firstRun.alreadyDefault")}</p>
  {:else if isDefault === false}
    <button
      type="button"
      onclick={async () => {
        await client.call({ type: "app.setDefaultBrowser" }).catch(() => {});
        await load();
      }}>{t("firstRun.makeDefault")}</button
    >
  {/if}
</section>
