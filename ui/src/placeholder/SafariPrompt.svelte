<script lang="ts">
  // "This site needs Safari to play" (spec 05 §11). Placeholder until the
  // design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Safari = Extract<PromptState, { type: "openInSafari" }>;
  const prompt = $derived(
    Object.values(store.state?.prompts ?? {}).find(
      (p): p is Safari =>
        !!p && p.type === "openInSafari" && store.state?.tabs[p.tabId]?.windowId === windowId,
    ),
  );

  const answer = (answer: "once" | "always" | "dismiss") => {
    if (prompt)
      void store.client
        .call({ type: "nav.openExternally", tabId: prompt.tabId, answer })
        .catch(() => {});
  };
</script>

{#if prompt}
  <div role="alertdialog" aria-labelledby="safari-title">
    <h2 id="safari-title">
      {t(
        prompt.reason === "applePay" ? "prompt.openInSafari.applePay" : "prompt.openInSafari.title",
        { host: prompt.host },
      )}
    </h2>
    <button type="button" onclick={() => answer("once")}>{t("prompt.openInSafari.open")}</button>
    <button type="button" onclick={() => answer("always")}>{t("prompt.openInSafari.always")}</button
    >
    <button type="button" onclick={() => answer("dismiss")}
      >{t("prompt.openInSafari.dismiss")}</button
    >
  </div>
{/if}
