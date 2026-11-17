<script lang="ts">
  // "Allow <name> to control Mado?" (spec 14 §2). Placeholder until the
  // design pass (M6), when it moves to the overlay.
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();

  type Card = Extract<PromptState, { type: "controlPairing" }>;
  const cards = $derived(
    Object.values(store.state?.prompts ?? {}).filter(
      (p): p is Card => p?.type === "controlPairing",
    ),
  );
  let chosen: Record<string, string[]> = $state({});
  const scopes = (c: Card) => [
    "read",
    "act",
    "tasks",
    "settings",
    ...(c.offerFull ? ["full"] : []),
  ];
  const picked = (c: Card) => chosen[c.id] ?? ["read"];
  const toggle = (c: Card, s: string, on: boolean) => {
    const now = picked(c).filter((x) => x !== s);
    chosen[c.id] = on ? [...now, s] : now;
  };
  const answer = (c: Card, allow: boolean) =>
    store.client
      .call({ type: "control.pairRespond", promptId: c.id, allow, scopes: allow ? picked(c) : [] })
      .catch(() => {});
</script>

{#each cards as c (c.id)}
  <div role="dialog" aria-label={t("control.pairing.title", { name: c.clientName })}>
    <h2>{t("control.pairing.title", { name: c.clientName })}</h2>
    <p>{t("control.pairing.version", { version: c.clientVersion })}</p>
    {#each scopes(c) as s (s)}
      <label
        ><input
          type="checkbox"
          checked={picked(c).includes(s)}
          onchange={(e) => toggle(c, s, e.currentTarget.checked)}
        />
        {t(`control.scope.${s}`)}</label
      >
    {/each}
    <button type="button" onclick={() => answer(c, true)}>{t("control.pairing.allow")}</button>
    <button type="button" onclick={() => answer(c, false)}>{t("control.pairing.deny")}</button>
  </div>
{/each}
