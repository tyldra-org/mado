<script lang="ts">
  // Settings › Developers: Mado in other agents (spec 14 §5). Set up and
  // Remove run the mado CLI's setup; Copy config gives the MCP entry for
  // any other client. Placeholder until M6.
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();
  type Target = { target: string; setUp: boolean; available: boolean };
  let targets: Target[] = $state([]);
  let config = $state("");
  let message = $state("");

  const load = () =>
    store.client
      .call({ type: "control.setupStatus" })
      .then((r: unknown) => {
        const v = r as { targets: Target[]; config: unknown };
        targets = v.targets;
        config = JSON.stringify(v.config, null, 2);
      })
      .catch(() => {});
  $effect(() => {
    void load();
  });
  const act = (target: string, remove: boolean) =>
    store.client.call({ type: "control.setup", target, remove }).then(
      (r: unknown) => {
        message = (r as { message: string }).message;
        void load();
      },
      (e: { error?: { messageKey?: string; args?: Record<string, string> } }) =>
        (message = t(e.error?.messageKey ?? "error.unknown", e.error?.args ?? {})),
    );
  const copy = () => navigator.clipboard?.writeText(config).catch(() => {});
</script>

<section data-testid="agent-setup">
  <h3>{t("control.setup.title")}</h3>
  {#if message}<p role="status">{message}</p>{/if}
  <ul>
    {#each targets as s (s.target)}
      <li>
        <strong>{t(`control.setup.${s.target}`)}</strong>
        {#if s.setUp}
          {t("control.setup.done")}
          <button type="button" onclick={() => act(s.target, true)}
            >{t("control.setup.remove")}</button
          >
        {:else if s.available}
          <button type="button" onclick={() => act(s.target, false)}
            >{t("control.setup.add")}</button
          >
        {:else}
          {t("control.setup.missing")}
        {/if}
      </li>
    {/each}
  </ul>
  <button type="button" disabled={!config} onclick={copy}>{t("control.setup.copy")}</button>
</section>
