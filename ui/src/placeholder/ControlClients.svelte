<script lang="ts">
  // Settings › Developers: paired apps and Revoke (spec 14 §2). The
  // switches are the setting rows above. Placeholder until M6.
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();
  type Client = { id: string; name: string; scopes: string[]; revoked: boolean };
  let clients: Client[] = $state([]);
  const load = () =>
    store.client
      .call({ type: "control.clients" })
      .then((r: unknown) => (clients = r as Client[]))
      .catch(() => {});
  $effect(() => {
    void store.state?.settings.values["developer.controlEnabled"];
    void load();
  });
  const revoke = (id: string) =>
    store.client
      .call({ type: "control.revoke", clientId: id })
      .then(load)
      .catch(() => {});
</script>

<section data-testid="control-clients">
  <h3>{t("control.clients")}</h3>
  {#if clients.length === 0}<p>{t("control.clients.none")}</p>{/if}
  <ul>
    {#each clients as c (c.id)}
      <li>
        <strong>{c.name}</strong>
        {c.scopes.map((s) => t(`control.scope.${s}`)).join(", ")}
        {#if c.revoked}
          · {t("control.clients.revoked")}
        {:else}
          <button type="button" onclick={() => revoke(c.id)}>{t("control.clients.revoke")}</button>
        {/if}
      </li>
    {/each}
  </ul>
</section>
