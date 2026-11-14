<script lang="ts">
  // "You have typed text that will be lost" before quitting or closing a
  // window (spec 04 §9). Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Unsaved = Extract<PromptState, { type: "unsavedText" }>;
  const prompt = $derived(
    Object.values(store.state?.prompts ?? {}).find(
      (p): p is Unsaved =>
        !!p && p.type === "unsavedText" && (p.windowId === null || p.windowId === windowId),
    ),
  );

  const answer = (proceed: boolean) => {
    if (prompt)
      void store.client
        .call({ type: "session.confirmClose", promptId: prompt.id, proceed })
        .catch(() => {});
  };
</script>

{#if prompt}
  <div role="alertdialog" aria-modal="true" aria-labelledby="unsaved-title">
    <h2 id="unsaved-title">
      {t(prompt.windowId ? "prompt.unsaved.titleWindow" : "prompt.unsaved.titleQuit")}
    </h2>
    <ul>
      {#each prompt.tabs as tab (tab.tabId)}
        <li>
          {tab.title} — {tab.host}
          <button
            type="button"
            onclick={() => {
              void store.client.call({ type: "tab.activate", tabId: tab.tabId }).catch(() => {});
              answer(false);
            }}>{t("prompt.unsaved.show")}</button
          >
        </li>
      {/each}
    </ul>
    <button type="button" onclick={() => answer(false)}>{t("prompt.unsaved.cancel")}</button>
    <button type="button" onclick={() => answer(true)}>
      {t(prompt.windowId ? "prompt.unsaved.closeAnyway" : "prompt.unsaved.quitAnyway")}
    </button>
  </div>
{/if}
