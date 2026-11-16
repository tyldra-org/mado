<script lang="ts">
  // "Which should block ads?" and "Is this the new tab page you expected?"
  // (spec 09 §8). Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import type { OverlapChoice } from "../generated/OverlapChoice";
  import type { PromptState } from "../generated/PromptState";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();

  type Overlap = Extract<PromptState, { type: "overlap" }>;
  type Takeover = Extract<PromptState, { type: "takeover" }>;
  const overlap = $derived(
    Object.values(store.state?.prompts ?? {}).find((p): p is Overlap => p?.type === "overlap"),
  );
  const takeover = $derived(
    Object.values(store.state?.prompts ?? {}).find((p): p is Takeover => p?.type === "takeover"),
  );

  const choose = (choice: OverlapChoice) =>
    overlap &&
    store.client
      .call({ type: "extension.overlapRespond", promptId: overlap.id, choice })
      .catch(() => {});
  const answer = (keep: boolean) =>
    takeover &&
    store.client
      .call({ type: "extension.takeoverRespond", promptId: takeover.id, keep })
      .catch(() => {});
</script>

{#if overlap}
  <div role="alertdialog" aria-labelledby="overlap-title" data-testid="overlap-prompt">
    <h2 id="overlap-title">{t(`prompt.overlap.${overlap.category}`, { name: overlap.name })}</h2>
    {#each overlap.choices as choice (choice)}
      <button type="button" onclick={() => choose(choice)}
        >{t(`prompt.overlap.choice.${choice}`, { name: overlap.name })}{choice === overlap.default
          ? ` (${t("prompt.overlap.recommended")})`
          : ""}</button
      >
    {/each}
  </div>
{:else if takeover}
  <div role="alertdialog" aria-labelledby="takeover-title" data-testid="takeover-prompt">
    <h2 id="takeover-title">{t(`prompt.takeover.${takeover.what}`, { name: takeover.name })}</h2>
    <button type="button" onclick={() => answer(true)}>{t("prompt.takeover.keep")}</button>
    <button type="button" onclick={() => answer(false)}>{t("prompt.takeover.changeBack")}</button>
  </div>
{/if}
