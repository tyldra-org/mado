<script lang="ts">
  // "Save password for <site>?" or "Update password" (spec 11 §5).
  // Placeholder until the design pass (M6). The username and password can
  // be edited before saving; a locked vault unlocks first.
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { SaveAction } from "../generated/SaveAction";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Save = Extract<PromptState, { type: "savePassword" }>;
  const prompt = $derived(
    Object.values(store.state?.prompts ?? {}).find(
      (p): p is Save => !!p && p.type === "savePassword" && p.windowId === windowId,
    ),
  );
  let username = $state("");
  let password = $state("");
  $effect(() => {
    username = prompt?.username ?? "";
    password = "";
  });

  const respond = async (action: SaveAction) => {
    if (!prompt) return;
    const cmd = {
      type: "vault.saveRespond" as const,
      promptId: prompt.id,
      action,
      username: username === prompt.username ? null : username,
      password: password || null,
    };
    try {
      await store.client.call(cmd);
    } catch (e) {
      if ((e as { error?: { code?: string } }).error?.code !== "locked") return;
      await store.client.call({ type: "vault.unlock" });
      await store.client.call(cmd);
    }
  };
</script>

{#if prompt}
  <div role="dialog" aria-labelledby="save-title">
    <h2 id="save-title">
      {t(prompt.update ? "prompt.save.update" : "prompt.save.title", { site: prompt.site })}
    </h2>
    <label>{t("prompt.save.username")} <input bind:value={username} /></label>
    <label
      >{t("prompt.save.password")}
      <input type="password" bind:value={password} placeholder="••••••••" /></label
    >
    <button type="button" onclick={() => respond("save")}
      >{t(prompt.update ? "prompt.save.updateButton" : "prompt.save.save")}</button
    >
    <button type="button" onclick={() => respond("notNow")}>{t("prompt.save.notNow")}</button>
    <button type="button" onclick={() => respond("never")}>{t("prompt.save.never")}</button>
  </div>
{/if}
