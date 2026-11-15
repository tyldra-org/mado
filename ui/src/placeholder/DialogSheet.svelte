<script lang="ts">
  // Page dialogs, sign-in, client certificates and the insecure form
  // warning (spec 07 §7), over the tab. Placeholder until the design pass
  // (M6), which draws them in the overlay over the page.
  import type { CoreStore } from "../core/store.svelte";
  import type { PromptState } from "../generated/PromptState";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Sheet = Extract<PromptState, { type: "dialog" | "auth" | "clientCert" | "insecureForm" }>;
  const sheet = $derived.by(() => {
    const active = store.state?.windows[windowId]?.activeTabId;
    return Object.values(store.state?.prompts ?? {}).find(
      (p): p is Sheet =>
        !!p &&
        (p.type === "dialog" ||
          p.type === "auth" ||
          p.type === "clientCert" ||
          p.type === "insecureForm") &&
        p.tabId === active,
    );
  });

  let text = $state("");
  let suppress = $state(false);
  let user = $state("");
  let password = $state("");
  let identity = $state("");

  $effect(() => {
    if (sheet?.type === "dialog") text = sheet.defaultText ?? "";
    if (sheet?.type === "clientCert") identity = sheet.identities[0]?.id ?? "";
    suppress = false;
    user = "";
    password = "";
  });

  const call = (cmd: Parameters<CoreStore["client"]["call"]>[0]) =>
    void store.client.call(cmd).catch(() => {});
  const host = (origin: string) => {
    try {
      return new URL(origin).host;
    } catch {
      return origin;
    }
  };
</script>

{#if sheet?.type === "dialog"}
  <div role="alertdialog" aria-labelledby="dialog-title">
    <h2 id="dialog-title">
      {sheet.kind === "beforeUnload"
        ? t("dialog.leave.title")
        : t("dialog.says", { host: host(sheet.origin) })}
    </h2>
    <p>{sheet.kind === "beforeUnload" ? t("dialog.leave.body") : sheet.message}</p>
    {#if sheet.kind === "prompt"}<input aria-label={t("dialog.answer")} bind:value={text} />{/if}
    {#if sheet.offerSuppress}
      <label><input type="checkbox" bind:checked={suppress} />{t("dialog.suppress")}</label>
    {/if}
    {#if sheet.kind !== "alert"}
      <button
        type="button"
        onclick={() =>
          call({ type: "prompt.answerDialog", id: sheet.id, accept: false, text: null, suppress })}
        >{t(sheet.kind === "beforeUnload" ? "dialog.stay" : "dialog.cancel")}</button
      >
    {/if}
    <button
      type="button"
      onclick={() =>
        call({
          type: "prompt.answerDialog",
          id: sheet.id,
          accept: true,
          text: sheet.kind === "prompt" ? text : null,
          suppress,
        })}>{t(sheet.kind === "beforeUnload" ? "dialog.leave.leave" : "dialog.ok")}</button
    >
  </div>
{:else if sheet?.type === "auth"}
  <div role="dialog" aria-labelledby="auth-title">
    <h2 id="auth-title">
      {t(sheet.isProxy ? "auth.proxyTitle" : "auth.title", { host: sheet.host })}
    </h2>
    {#if sheet.realm}<p>{sheet.realm}</p>{/if}
    <label>{t("auth.user")} <input autocomplete="username" bind:value={user} /></label>
    <label
      >{t("auth.password")}
      <input type="password" autocomplete="current-password" bind:value={password} /></label
    >
    <button
      type="button"
      onclick={() => call({ type: "prompt.answerAuth", id: sheet.id, user: null, password: null })}
      >{t("dialog.cancel")}</button
    >
    <button
      type="button"
      onclick={() => call({ type: "prompt.answerAuth", id: sheet.id, user, password })}
      >{t("auth.signIn")}</button
    >
  </div>
{:else if sheet?.type === "clientCert"}
  <div role="dialog" aria-labelledby="cert-title">
    <h2 id="cert-title">{t("clientCert.title", { host: sheet.host })}</h2>
    <select aria-label={t("clientCert.choose")} bind:value={identity}>
      {#each sheet.identities as i (i.id)}<option value={i.id}>{i.label}</option>{/each}
    </select>
    <button
      type="button"
      onclick={() => call({ type: "prompt.answerClientCert", id: sheet.id, identity: null })}
      >{t("clientCert.none")}</button
    >
    <button
      type="button"
      onclick={() => call({ type: "prompt.answerClientCert", id: sheet.id, identity })}
      >{t("dialog.ok")}</button
    >
  </div>
{:else if sheet?.type === "insecureForm"}
  <div role="alertdialog" aria-labelledby="form-title">
    <h2 id="form-title">{t("insecureForm.title")}</h2>
    <p>{t("insecureForm.body")}</p>
    <button
      type="button"
      onclick={() => call({ type: "prompt.respond", id: sheet.id, decision: "never" })}
      >{t("dialog.cancel")}</button
    >
    <button
      type="button"
      onclick={() => call({ type: "prompt.respond", id: sheet.id, decision: "allowThisTime" })}
      >{t("insecureForm.send")}</button
    >
  </div>
{/if}
