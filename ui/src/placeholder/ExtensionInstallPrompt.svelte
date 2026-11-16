<script lang="ts">
  // "Add <extension>?" (spec 09 §2.2 step 7): what it may read and use,
  // and what Mado lacks. Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import type { ExtensionInstallPrompt } from "../generated/ExtensionInstallPrompt";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  const prompt = $derived.by(() => {
    for (const p of Object.values(store.state?.prompts ?? {})) {
      if (p?.type === "extensionInstall" && (p.windowId === null || p.windowId === windowId)) {
        return p as ExtensionInstallPrompt & { type: "extensionInstall" };
      }
    }
    return undefined;
  });

  const answer = (add: boolean) => {
    if (!prompt) return;
    void store.client
      .call({ type: "extension.confirmInstall", promptId: prompt.id, add })
      .catch(() => {});
  };
</script>

{#if prompt}
  <div role="alertdialog" aria-labelledby="extension-title" data-testid="extension-install">
    {#if prompt.icon}<img src={prompt.icon} alt="" width="32" height="32" />{/if}
    <h2 id="extension-title">{t("prompt.extension.title", { name: prompt.name })}</h2>
    {#if prompt.olderFormat}<p>{t("prompt.extension.olderFormat")}</p>{/if}
    <ul>
      {#if prompt.allSites}
        <li>{t("prompt.extension.allSites")}</li>
      {:else if prompt.hosts.length > 0}
        <li>{t("prompt.extension.sites", { hosts: prompt.hosts.join(", ") })}</li>
      {/if}
      {#if prompt.permissions.length > 0}
        <li>{t("prompt.extension.permissions", { permissions: prompt.permissions.join(", ") })}</li>
      {/if}
      {#if prompt.optional.length > 0}
        <li>{t("prompt.extension.optional", { optional: prompt.optional.join(", ") })}</li>
      {/if}
      {#each prompt.unavailable as feature (feature)}
        <li>{t("prompt.extension.unavailable", { feature })}</li>
      {/each}
    </ul>
    {#if prompt.mayNotWork}<p>{t("prompt.extension.mayNotWork")}</p>{/if}
    {#if prompt.limitation}<p>{t(prompt.limitation, { name: prompt.name })}</p>{/if}
    {#if prompt.replaces}<p>{t("prompt.extension.replaces", { version: prompt.replaces })}</p>{/if}
    <button type="button" onclick={() => answer(true)}>{t("prompt.extension.add")}</button>
    <button type="button" onclick={() => answer(false)}>{t("prompt.extension.cancel")}</button>
  </div>
{/if}
