<script lang="ts">
  // Extension buttons (spec 09 §6): pinned ones in the toolbar, the rest in
  // the extensions menu. A click opens the popup under the button. Placeholder
  // until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import type { ExtensionButton } from "../generated/ExtensionButton";
  import type { WindowId } from "../generated/WindowId";
  import Icon from "../design/Icon.svelte";
  import { portal } from "../design/portal";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  const pinned = $derived(new Set(store.state?.extensions?.pinned ?? []));
  const buttons = $derived(store.state?.extensions?.toolbar ?? []);
  let menu = $state(false);

  const click = (b: ExtensionButton, el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    menu = false;
    void store.client
      .call({
        type: "extension.openPopup",
        id: b.id,
        windowId,
        anchor: { x: r.x, y: r.y, width: r.width, height: r.height },
      })
      .catch(() => {});
  };
</script>

{#if buttons.length > 0}
  <div role="toolbar" aria-label={t("extensions.toolbar")} data-testid="extension-toolbar">
    {#each buttons.filter((b) => pinned.has(b.id)) as b (b.id)}
      <button
        type="button"
        title={b.title}
        disabled={!b.enabled}
        data-access={b.siteAccessForActiveTab}
        onclick={(e) => click(b, e.currentTarget)}
      >
        {#if b.iconUrl}<img src={b.iconUrl} alt="" width="16" height="16" />{:else}{b.name.slice(
            0,
            1,
          )}{/if}
        {#if b.badgeText}<span class="badge">{b.badgeText}</span>{/if}
      </button>
    {/each}
    <button
      type="button"
      class="icon"
      aria-expanded={menu}
      aria-label={t("extensions.menu")}
      title={t("extensions.menu")}
      onclick={() => (menu = !menu)}><Icon name="puzzle" /></button
    >
    {#if menu}
      <ul class="popover card floating menu" role="menu" {@attach portal("popovers")}>
        {#each buttons as b (b.id)}
          <li role="none">
            <button type="button" role="menuitem" onclick={(e) => click(b, e.currentTarget)}
              >{b.name}</button
            >
            <button
              type="button"
              aria-pressed={pinned.has(b.id)}
              onclick={() =>
                store.client
                  .call({ type: "extension.pin", id: b.id, pinned: !pinned.has(b.id) })
                  .catch(() => {})}
              >{pinned.has(b.id) ? t("extensions.unpin") : t("extensions.pin")}</button
            >
          </li>
        {/each}
        <li role="none">
          <button
            type="button"
            role="menuitem"
            onclick={() =>
              store.client
                .call({ type: "app.openSettings", section: "extensions" })
                .catch(() => {})}>{t("extensions.manage")}</button
          >
        </li>
      </ul>
    {/if}
  </div>
{/if}
