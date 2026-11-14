<script lang="ts">
  // "Page unresponsive" with Wait and Exit page (spec 02 §5.3). Placeholder
  // until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Unresponsive = Extract<PromptState, { type: "unresponsive" }>;
  const prompt = $derived(
    Object.values(store.state?.prompts ?? {}).find(
      (p): p is Unresponsive =>
        !!p && p.type === "unresponsive" && store.state?.tabs[p.tabId]?.windowId === windowId,
    ),
  );

  const answer = (exit: boolean) => {
    if (prompt)
      void store.client
        .call({ type: "tab.answerUnresponsive", promptId: prompt.id, exit })
        .catch(() => {});
  };
</script>

{#if prompt}
  <div role="alertdialog" aria-labelledby="unresponsive-title">
    <h2 id="unresponsive-title">{t("prompt.unresponsive.title", { title: prompt.title })}</h2>
    <button type="button" onclick={() => answer(false)}>{t("prompt.unresponsive.wait")}</button>
    <button type="button" onclick={() => answer(true)}>{t("prompt.unresponsive.exit")}</button>
  </div>
{/if}
