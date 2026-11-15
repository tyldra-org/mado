<script lang="ts">
  // A site's permission request (spec 07 §3). Placeholder until the design
  // pass (M6), which draws it in the overlay under the address bar. Buttons
  // do nothing before `armedAt`; the core refuses them too.
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Permission = Extract<PromptState, { type: "permission" }>;
  const prompt = $derived.by(() => {
    const active = store.state?.windows[windowId]?.activeTabId;
    return Object.values(store.state?.prompts ?? {}).find(
      (p): p is Permission => !!p && p.type === "permission" && p.tabId === active,
    );
  });
  const host = $derived.by(() => {
    try {
      return prompt ? new URL(prompt.origin).host : "";
    } catch {
      return prompt?.origin ?? "";
    }
  });

  const respond = (decision: "allowWhileVisiting" | "allowThisTime" | "never") => {
    if (prompt && Date.now() >= prompt.armedAt)
      void store.client.call({ type: "prompt.respond", id: prompt.id, decision }).catch(() => {});
  };
  const dismiss = () => {
    if (prompt) void store.client.call({ type: "prompt.dismiss", id: prompt.id }).catch(() => {});
  };
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && prompt && dismiss()} />

{#if prompt}
  <div role="alertdialog" aria-labelledby="permission-title">
    <h2 id="permission-title">
      {t("prompt.permission.title", {
        host,
        what: prompt.keys.map((k) => t(`permission.${k}`)).join(", "),
      })}
    </h2>
    <button type="button" onclick={() => respond("allowWhileVisiting")}
      >{t("prompt.permission.allowWhileVisiting")}</button
    >
    <button type="button" onclick={() => respond("allowThisTime")}
      >{t("prompt.permission.allowThisTime")}</button
    >
    <button type="button" onclick={() => respond("never")}>{t("prompt.permission.never")}</button>
  </div>
{/if}
