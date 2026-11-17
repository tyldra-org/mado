<script lang="ts">
  // The passkey sheets (spec 11 §8): create ("Create a passkey for
  // <rpId>?") and sign in (choose an account). Placeholder until the design
  // pass (M6). Save and Use ask for Touch ID in the core.
  import type { CoreStore } from "../core/store.svelte";
  import type { PasskeyAction } from "../generated/PasskeyAction";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Sheet = Extract<PromptState, { type: "passkey" }>;
  const sheet = $derived(
    Object.values(store.state?.prompts ?? {}).find(
      (p): p is Sheet => !!p && p.type === "passkey" && p.windowId === windowId,
    ),
  );
  const respond = (action: PasskeyAction, itemId: string | null = null) => {
    if (sheet)
      void store.client
        .call({ type: "vault.passkeyRespond", promptId: sheet.id, action, itemId })
        .catch(() => {});
  };
  const unlock = () => void store.client.call({ type: "vault.unlock" }).catch(() => {});
</script>

{#if sheet}
  <div role="dialog" aria-labelledby="passkey-title">
    <h2 id="passkey-title">
      {t(sheet.mode === "create" ? "passkey.create.title" : "passkey.get.title", {
        site: sheet.rpId,
      })}
    </h2>
    {#if sheet.locked}
      <button type="button" onclick={unlock}>{t("passkey.unlock")}</button>
    {:else if sheet.mode === "create"}
      <p>{sheet.userName}</p>
      <button type="button" onclick={() => respond("save")}>{t("passkey.save")}</button>
    {:else}
      {#each sheet.accounts as a (a.itemId)}
        <button type="button" onclick={() => respond("use", a.itemId)}
          >{a.userName}{#if a.userDisplayName && a.userDisplayName !== a.userName}<small>
              {a.userDisplayName}</small
            >{/if}</button
        >
      {/each}
    {/if}
    <button type="button" onclick={() => respond("otherDevice")}>{t("passkey.otherDevice")}</button>
    <p>{t("passkey.otherDevice.note")}</p>
    <button type="button" onclick={() => respond("cancel")}>{t("passkey.cancel")}</button>
  </div>
{/if}
