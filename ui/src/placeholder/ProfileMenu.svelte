<script lang="ts">
  // The profile menu in the window's toolbar (spec 12 §1): the profiles,
  // to open one, and Manage profiles. Placeholder until M6.
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";

  type Profile = { id: string; name: string; current: boolean };
  let { store }: { store: CoreStore } = $props();
  let open = $state(false);
  let profiles = $state<Profile[]>([]);

  async function toggle() {
    open = !open;
    if (open)
      profiles = (await store.client.call({ type: "profile.list" }).catch(() => [])) as Profile[];
  }
</script>

<button type="button" aria-expanded={open} onclick={toggle}>
  {store.state?.profile.name ?? t("app.name")}
</button>
{#if open}
  <ul aria-label={t("profiles.menu")}>
    {#each profiles as p (p.id)}
      <li>
        {#if p.current}
          <span aria-current="true">{p.name}</span>
        {:else}
          <button
            type="button"
            onclick={() => (
              (open = false),
              store.client.call({ type: "profile.open", id: p.id }).catch(() => {})
            )}>{p.name}</button
          >
        {/if}
      </li>
    {/each}
  </ul>
{/if}
