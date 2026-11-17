<script lang="ts">
  // Settings › Passwords: the password managers Mado fills from (spec 11
  // §6) and what each needs. Placeholder until the design pass (M6); the
  // switches are the `passwords.onePassword` and `passwords.bitwarden`
  // rows above.
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();
  type Source = { id: string; enabled: boolean; status: string | { error: string } };
  let sources: Source[] = $state([]);
  const load = () =>
    store.client
      .call({ type: "vault.sources" })
      .then((r: unknown) => (sources = r as Source[]))
      .catch(() => {});
  $effect(() => {
    void store.state?.settings.values["passwords.onePassword"];
    void store.state?.settings.values["passwords.bitwarden"];
    void store.state?.vault?.status;
    void load();
  });
  let master = $state("");
  let message = $state("");
  const unlockBitwarden = () =>
    store.client.call({ type: "vault.bitwardenUnlock", password: master }).then(
      () => {
        master = "";
        message = "";
        void load();
      },
      (e: { error?: { messageKey?: string } }) =>
        (message = t(e.error?.messageKey ?? "error.unknown")),
    );
  const lockBitwarden = () =>
    store.client
      .call({ type: "vault.bitwardenLock" })
      .then(load)
      .catch(() => {});
  const label = (s: Source) =>
    typeof s.status === "string"
      ? t(`passwords.source.status.${s.status}`)
      : t(`passwords.source.error.${s.status.error}`);
</script>

<section data-testid="password-sources">
  <h3>{t("passwords.sources")}</h3>
  <ul>
    {#each sources as s (s.id)}
      <li>
        {t(`fill.source.${s.id}`)}: {label(s)}
        {#if s.id === "onePassword" && s.enabled && s.status !== "ready"}
          <p>{t("passwords.onePassword.setup")}</p>
        {/if}
        {#if s.id === "bitwarden" && s.enabled && s.status === "locked"}
          {#if message}<p role="alert">{message}</p>{/if}
          <label
            >{t("passwords.bitwarden.master")}
            <input type="password" bind:value={master} autocomplete="off" /></label
          >
          <button type="button" disabled={!master} onclick={unlockBitwarden}
            >{t("passwords.bitwarden.unlock")}</button
          >
        {:else if s.id === "bitwarden" && s.enabled && s.status === "ready"}
          <button type="button" onclick={lockBitwarden}>{t("passwords.bitwarden.lock")}</button>
        {:else if s.id === "bitwarden" && s.enabled && s.status !== "ready"}
          <p>{t("passwords.bitwarden.setup")}</p>
        {/if}
      </li>
    {/each}
  </ul>
  <button type="button" onclick={load}>{t("passwords.sources.refresh")}</button>
</section>
