<script lang="ts">
  // An agent's action that needs the person (spec 13 §9.3): the action in
  // plain words, the site, the total for a payment; Allow once, Allow for
  // this task (not for payments and deletes), Deny. Placeholder until M6.
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();
  type Card = Extract<PromptState, { type: "aiApproval" }>;
  const cards = $derived(
    Object.values(store.state?.prompts ?? {}).filter(
      (p): p is Card =>
        p?.type === "aiApproval" && (p.windowId === null || p.windowId === windowId),
    ),
  );
  const answer = (c: Card, a: string) =>
    store.client.call({ type: "ai.approval.respond", promptId: c.id, answer: a }).catch(() => {});
</script>

{#each cards as c (c.id)}
  <div class="approval" role="alertdialog" aria-label={t("ai.approval.title")}>
    <h2>{t("ai.approval.title")}</h2>
    <p>{c.detail}</p>
    <p>{c.site}</p>
    {#if c.amount}<p class="amount">{t("ai.approval.amount", { amount: c.amount })}</p>{/if}
    <button type="button" onclick={() => answer(c, "once")}>{t("ai.approval.once")}</button>
    {#if c.allowForTask}
      <button type="button" onclick={() => answer(c, "task")}>{t("ai.approval.task")}</button>
    {/if}
    <button type="button" onclick={() => answer(c, "deny")}>{t("ai.approval.deny")}</button>
  </div>
{/each}
