<script lang="ts">
  // The fill menu at a form field (spec 11 §4.1). Placeholder until the
  // design pass (M6) draws it in the overlay at `rect`. Choosing an item
  // sends `vault.fill`; the click itself is the gesture the core needs.
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Menu = Extract<PromptState, { type: "fillMenu" }>;
  const menu = $derived(
    Object.values(store.state?.prompts ?? {}).find(
      (p): p is Menu => !!p && p.type === "fillMenu" && p.windowId === windowId,
    ),
  );
  let message = $state("");

  const fill = (itemRef: string) => {
    if (!menu) return;
    store.client
      .call({ type: "vault.fill", tabId: menu.tabId, formKey: menu.formKey, itemRef })
      .then(
        () => (message = ""),
        (e: { messageKey?: string }) => (message = t(e.messageKey ?? "error.unknown")),
      );
  };
  const unlock = () => void store.client.call({ type: "vault.unlock" }).catch(() => {});
  const dismiss = () => {
    if (menu) void store.client.call({ type: "prompt.dismiss", id: menu.id }).catch(() => {});
  };
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && menu && dismiss()} />

{#if menu}
  <div
    role="menu"
    aria-label={t("fill.menu")}
    style:left="{menu.rect.x}px"
    style:top="{menu.rect.y + menu.rect.height}px"
  >
    {#if message}<p role="alert">{message}</p>{/if}
    {#if menu.locked}
      <button type="button" role="menuitem" onclick={unlock}>{t("fill.unlock")}</button>
    {:else if menu.loading}
      <p>{t("fill.loading")}</p>
    {:else if menu.items.length === 0}
      <p>{t("fill.none")}</p>
    {:else}
      {#each menu.items as item (item.itemRef)}
        <button type="button" role="menuitem" onclick={() => fill(item.itemRef)}>
          {item.title}{#if item.subtitle}<small> {item.subtitle}</small>{/if}
        </button>
      {/each}
    {/if}
  </div>
{/if}
