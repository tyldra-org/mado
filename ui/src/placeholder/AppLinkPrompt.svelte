<script lang="ts">
  // "Open <app>?" for links of other apps, and a site asking to handle a
  // scheme (spec 07 §6). Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Link = Extract<PromptState, { type: "appLink" } | { type: "protocolHandler" }>;
  const prompt = $derived.by(() => {
    const active = store.state?.windows[windowId]?.activeTabId;
    return Object.values(store.state?.prompts ?? {}).find(
      (p): p is Link =>
        !!p && (p.type === "appLink" || p.type === "protocolHandler") && p.tabId === active,
    );
  });
  const host = $derived.by(() => {
    try {
      return prompt ? new URL(prompt.origin).host : "";
    } catch {
      return prompt?.origin ?? "";
    }
  });
  let always = $state(false);

  const respond = (open: boolean) => {
    if (!prompt || Date.now() < prompt.armedAt) return;
    const decision = !open
      ? "never"
      : always || prompt.type === "protocolHandler"
        ? "allowWhileVisiting"
        : "allowThisTime";
    void store.client.call({ type: "prompt.respond", id: prompt.id, decision }).catch(() => {});
    always = false;
  };
</script>

{#if prompt?.type === "appLink"}
  <div role="alertdialog" aria-labelledby="applink-title">
    {#if prompt.appIcon}<img src={prompt.appIcon} alt="" width="32" height="32" />{/if}
    <h2 id="applink-title">{t("prompt.appLink.title", { app: prompt.appName })}</h2>
    <label>
      <input type="checkbox" bind:checked={always} />
      {t("prompt.appLink.always", { host, app: prompt.appName })}
    </label>
    <button type="button" onclick={() => respond(true)}
      >{t("prompt.appLink.open", { app: prompt.appName })}</button
    >
    <button type="button" onclick={() => respond(false)}>{t("prompt.appLink.cancel")}</button>
  </div>
{:else if prompt?.type === "protocolHandler"}
  <div role="alertdialog" aria-labelledby="handler-title">
    <h2 id="handler-title">{t("prompt.protocolHandler.title", { host, scheme: prompt.scheme })}</h2>
    <button type="button" onclick={() => respond(true)}>{t("prompt.protocolHandler.allow")}</button>
    <button type="button" onclick={() => respond(false)}>{t("prompt.protocolHandler.block")}</button
    >
  </div>
{/if}
