<script lang="ts">
  // The profile menu in the window's toolbar (spec 12 §1): the profiles,
  // to open one, and Manage profiles. Placeholder until M6.
  import type { CoreStore } from "../core/store.svelte";
  import Icon from "../design/Icon.svelte";
  import { portal } from "../design/portal";
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

<button type="button" class="quiet profile" aria-expanded={open} onclick={toggle}>
  <span class="avatar" data-color={store.state?.profile.color}
    >{(store.state?.profile.name ?? t("app.name")).charAt(0)}</span
  >
  <span>{store.state?.profile.name ?? t("app.name")}</span>
</button>
{#if open}
  <ul
    class="popover card floating menu"
    aria-label={t("profiles.menu")}
    {@attach portal("popovers")}
  >
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
    {#if !store.state?.profile.isPrivate}
      <li>
        <button
          type="button"
          onclick={() => (
            (open = false),
            store.client.call({ type: "window.new", private: true, urls: [] }).catch(() => {})
          )}>{t("profiles.privateWindow")}</button
        >
      </li>
    {/if}
    <li>
      <button
        type="button"
        onclick={() => (
          (open = false),
          store.client.call({ type: "profile.openGuest" }).catch(() => {})
        )}>{t("profiles.guestWindow")}</button
      >
    </li>
  </ul>
{/if}

<style>
  .profile {
    gap: var(--space-3);
    padding: 0 var(--space-3);
    color: var(--text-on-chrome);
    font-weight: var(--weight-regular);
  }
  .avatar {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--accent);
    color: var(--accent-text);
    font-size: var(--text-caption);
    font-weight: var(--weight-strong);
  }
</style>
