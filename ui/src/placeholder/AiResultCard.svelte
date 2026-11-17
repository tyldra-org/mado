<script lang="ts">
  // A page command's result card (spec 13 §6): Copy, Replace, Insert below,
  // Retry and Retry with another model. Placeholder until the design pass
  // (M6), when it moves to the overlay.
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Card = Extract<PromptState, { type: "aiResult" }>;
  type Model = { connectionId: string; modelId: string; name: string; show: boolean };
  const cards = $derived(
    Object.values(store.state?.prompts ?? {}).filter(
      (p): p is Card => p?.type === "aiResult" && p.windowId === windowId,
    ),
  );
  let models: Model[] = $state([]);
  let other = $state("");
  const respond = (card: Card, action: string, model: string | null = null) =>
    store.client
      .call({ type: "ai.command.respond", promptId: card.id, action, model })
      .catch(() => {});
  const loadModels = () =>
    store.client
      .call({ type: "ai.model.list" })
      .then((r: unknown) => (models = (r as Model[]).filter((m) => m.show)))
      .catch(() => {});
  const label = (name: string) => (name.startsWith("ai.command.") ? t(name) : name);
</script>

{#each cards as card (card.id)}
  <div role="dialog" aria-label={label(card.name)}>
    <h2>{label(card.name)}</h2>
    {#if card.error}
      <p role="alert">{t(`error.ai.${card.error}`)}</p>
    {:else}
      <p aria-live="polite">{card.text || t("ai.command.working")}</p>
    {/if}
    {#if card.model === "onDevice"}<small>{t("ai.command.onDevice")}</small>{/if}
    <button type="button" disabled={!card.done || !card.text} onclick={() => respond(card, "copy")}
      >{t("ai.command.copy")}</button
    >
    {#if card.editable}
      <button
        type="button"
        disabled={!card.done || !card.text}
        onclick={() => respond(card, "replace")}>{t("ai.command.replace")}</button
      >
      <button
        type="button"
        disabled={!card.done || !card.text}
        onclick={() => respond(card, "insert")}>{t("ai.command.insert")}</button
      >
    {/if}
    <button type="button" disabled={!card.done} onclick={() => respond(card, "retry")}
      >{t("ai.command.retry")}</button
    >
    <label
      >{t("ai.command.retryWith")}
      <select bind:value={other} onfocus={loadModels}>
        <option value="">—</option>
        {#each models as m (`${m.connectionId}:${m.modelId}`)}
          <option value={`${m.connectionId}:${m.modelId}`}>{m.name}</option>
        {/each}
      </select></label
    >
    <button
      type="button"
      disabled={!card.done || !other}
      onclick={() => respond(card, "retry", other)}>{t("ai.command.retryWithGo")}</button
    >
    <button type="button" onclick={() => respond(card, "close")}>{t("ai.command.close")}</button>
  </div>
{/each}
