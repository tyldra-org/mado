<script lang="ts">
  // Settings › AI › Models: subscription CLIs (spec 13 §2.3). Placeholder
  // until the design pass (M6). Install and login are the vendors' own
  // commands, shown before Mado runs them in the person's terminal; Mado
  // looks again every 5 seconds while this is open.
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();

  type Cli = {
    provider: string;
    name: string;
    version: string | null;
    needsNode: boolean;
    nodeOk: boolean;
    install: string;
    login: string;
    loginHint: string | null;
    ready: boolean;
  };
  let clis: Cli[] = $state([]);
  let message = $state("");
  const connected = $derived(
    new Set(Object.values(store.state?.ai?.connections ?? {}).map((c) => c.provider)),
  );

  const look = () =>
    store.client
      .call({ type: "ai.cli.list" })
      .then((r: unknown) => (clis = r as Cli[]))
      .catch(() => {});
  $effect(() => {
    void look();
    const every = setInterval(look, 5000);
    return () => clearInterval(every);
  });
  const fail = (e: { error?: { messageKey?: string } }) =>
    (message = t(e.error?.messageKey ?? "error.unknown"));
  const runIt = (provider: string, what: "install" | "login") =>
    store.client.call({ type: "ai.cli.run", provider, what }).then(() => (message = ""), fail);
  const connect = (provider: string) =>
    store.client
      .call({
        type: "ai.connection.add",
        kind: "subscriptionCli",
        provider,
        label: null,
        baseUrl: null,
        secret: null,
      })
      .then(() => (message = ""), fail);
</script>

<section data-testid="ai-clis">
  <h4>{t("ai.cli.title")}</h4>
  <p>{t("ai.cli.help")}</p>
  {#if message}<p role="alert">{message}</p>{/if}
  <ul>
    {#each clis as c (c.provider)}
      <li>
        <strong>{c.name}</strong>
        {c.version ? t("ai.cli.installed", { version: c.version }) : t("ai.cli.missing")}
        {#if !c.version}
          <code>{t("ai.cli.command", { command: c.install })}</code>
          <button type="button" onclick={() => runIt(c.provider, "install")}
            >{t("ai.cli.install")}</button
          >
        {:else}
          <code>{t("ai.cli.command", { command: c.login })}</code>
          {#if c.loginHint}<small>{c.loginHint}</small>{/if}
          <button type="button" onclick={() => runIt(c.provider, "login")}
            >{t("ai.cli.login")}</button
          >
          {#if c.needsNode && !c.nodeOk}
            <p>{t("error.ai.needsNode")}</p>
          {:else if c.ready && !connected.has(c.provider)}
            <button type="button" onclick={() => connect(c.provider)}
              >{t("ai.cli.connect", { name: c.name })}</button
            >
          {/if}
        {/if}
      </li>
    {/each}
  </ul>
</section>
