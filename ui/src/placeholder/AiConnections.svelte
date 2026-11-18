<script lang="ts">
  // Settings › AI › Models: the AI connections (spec 13 §2). Placeholder
  // until the design pass (M6). Keys go to the core once and are shown
  // only by their last four characters.
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";
  import AiClis from "./AiClis.svelte";

  let { store }: { store: CoreStore } = $props();

  type Found = { provider: string; baseUrl: string; models: number };
  const keyed = [
    "anthropic",
    "openai",
    "google",
    "xai",
    "mistral",
    "deepseek",
    "groq",
    "openrouter",
    "cloudflare",
    "openaiCompatible",
  ];
  const needsUrl = new Set(["cloudflare", "openaiCompatible"]);

  const on = $derived(store.state?.features?.ai === true);
  const connections = $derived(
    Object.values(store.state?.ai?.connections ?? {}).sort(
      (a, b) => a.priority - b.priority || a.id.localeCompare(b.id),
    ),
  );
  let provider = $state("anthropic");
  let baseUrl = $state("");
  let secret = $state("");
  let message = $state("");
  let local: Found[] = $state([]);
  let replacing: Record<string, string> = $state({});

  const fail = (e: { error?: { messageKey?: string } }) =>
    (message = t(e.error?.messageKey ?? "error.unknown"));
  const call = (cmd: Parameters<typeof store.client.call>[0]) =>
    store.client.call(cmd).then(() => (message = ""), fail);

  const detect = () =>
    store.client
      .call({ type: "ai.connection.detectLocal" })
      .then((r: unknown) => (local = r as Found[]))
      .catch(() => {});
  $effect(() => {
    if (on) void detect();
  });

  const add = () =>
    call({
      type: "ai.connection.add",
      kind: "apiKey",
      provider,
      label: null,
      baseUrl: needsUrl.has(provider) ? baseUrl : null,
      secret,
    }).then(() => {
      secret = "";
      baseUrl = "";
    });
  const signIn = () =>
    call({
      type: "ai.connection.add",
      kind: "oauth",
      provider: "openrouter",
      label: null,
      baseUrl: null,
      secret: null,
    });
  const useLocal = (f: Found) =>
    call({
      type: "ai.connection.add",
      kind: "local",
      provider: f.provider,
      label: null,
      baseUrl: f.baseUrl,
      secret: null,
    }).then(detect);
  const test = (id: string) => call({ type: "ai.connection.test", id });
  const remove = (id: string) => call({ type: "ai.connection.remove", id }).then(detect);
  const up = (id: string, priority: number) =>
    call({ type: "ai.connection.update", id, label: null, priority: priority - 2, secret: null });
  const replace = (id: string) =>
    call({
      type: "ai.connection.update",
      id,
      label: null,
      priority: null,
      secret: replacing[id] ?? "",
    }).then(() => (replacing[id] = ""));
</script>

<section data-testid="ai-connections">
  <h3>{t("ai.connections")}</h3>
  {#if !on}
    <p>{t("ai.connections.off")}</p>
  {:else}
    {#if message}<p role="alert">{message}</p>{/if}
    <ul>
      {#each connections as c, i (c.id)}
        {@const above = connections[i - 1]}
        <li>
          <strong>{c.label}</strong>
          {t(`ai.provider.${c.provider}`)} ·
          {t(`ai.connection.status.${c.status}`)}
          {#if c.error}· {t(`error.ai.${c.error}`)}{/if}
          {#if c.secretHint}· ••••{c.secretHint}{/if}
          {#if c.status === "ready"}· {t("ai.connection.models", { count: c.models })}{/if}
          <button type="button" onclick={() => test(c.id)}>{t("ai.connection.test")}</button>
          {#if above}
            <button type="button" onclick={() => up(c.id, above.priority + 1)}
              >{t("ai.connection.up")}</button
            >
          {/if}
          <button type="button" onclick={() => remove(c.id)}>{t("ai.connection.remove")}</button>
          {#if c.kind === "apiKey"}
            <label
              >{t("ai.connection.replaceKey")}
              <input type="password" autocomplete="off" bind:value={replacing[c.id]} /></label
            >
            <button type="button" disabled={!replacing[c.id]} onclick={() => replace(c.id)}
              >{t("ai.connection.saveKey")}</button
            >
          {/if}
        </li>
      {/each}
    </ul>
    <h4>{t("ai.connection.add")}</h4>
    <label
      >{t("ai.connection.provider")}
      <select bind:value={provider}>
        {#each keyed as p (p)}
          <option value={p}>{t(`ai.provider.${p}`)}</option>
        {/each}
      </select></label
    >
    {#if needsUrl.has(provider)}
      <label>{t("ai.connection.baseUrl")} <input type="url" bind:value={baseUrl} /></label>
    {/if}
    <label
      >{t("ai.connection.key")}
      <input type="password" autocomplete="off" bind:value={secret} /></label
    >
    <button type="button" disabled={!secret} onclick={add}>{t("ai.connection.addKey")}</button>
    <button type="button" onclick={signIn}>{t("ai.connection.signInOpenRouter")}</button>
    {#each local as f (f.baseUrl)}
      <button type="button" onclick={() => useLocal(f)}
        >{t("ai.connection.useLocal", { name: t(`ai.provider.${f.provider}`) })}</button
      >
    {/each}
    <AiClis {store} />
  {/if}
</section>
