<script lang="ts">
  // Settings › Passwords (spec 11): turning Mado Passwords on, unlocking
  // and recovery, the item list with search, details, copy and reveal after
  // Touch ID, editing, deleting and Recently deleted, two-factor codes,
  // health, import and export. Placeholder until the design pass (M6);
  // every vault command is reachable from here.
  import { CoreError } from "../core/client";
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();
  const call = (cmd: Record<string, unknown>) => store.client.call(cmd as never);

  type Summary = {
    id: string;
    kind: string;
    title: string;
    username: string | null;
    urls: string[];
    deletedAt: number | null;
  };
  type Detail = {
    id: string;
    item: Record<string, unknown> & { kind: string };
    code?: string;
    codeRemaining?: number;
    revealed: boolean;
  };
  type HealthItem = { id: string; title: string; username: string };
  type Health = { reused: HealthItem[][]; weak: HealthItem[]; leaked: HealthItem[] | null };

  const status = $derived(store.state?.vault?.status ?? "none");
  let message = $state("");
  let secret = $state("");
  let recoverWith = $state("");
  let items: Summary[] = $state([]);
  let deleted: Summary[] = $state([]);
  let query = $state("");
  let detail: Detail | null = $state(null);
  let editing: { username: string; password: string } | null = $state(null);
  let adding = $state({ url: "", username: "", password: "" });
  let health: Health | null = $state(null);
  let pending: string | null = $state(null);
  let files = $state({ csvIn: "", csvOut: "", vaultPath: "", passphrase: "", again: "" });

  const shown = $derived(
    items.filter((i) => {
      const q = query.trim().toLowerCase();
      return (
        !q ||
        i.title.toLowerCase().includes(q) ||
        (i.username ?? "").toLowerCase().includes(q) ||
        i.urls.some((u) => u.toLowerCase().includes(q))
      );
    }),
  );

  const fail = (e: unknown) => {
    message = t(
      e instanceof CoreError ? e.error.messageKey : "error.unknown",
      e instanceof CoreError ? (e.error.args as Record<string, string>) : {},
    );
  };
  const run = async (cmd: Record<string, unknown>) => {
    try {
      const r = await call(cmd);
      message = "";
      return r;
    } catch (e) {
      fail(e);
      return undefined;
    }
  };

  async function load() {
    if (status !== "unlocked") return;
    items = ((await run({ type: "vault.query" })) as Summary[] | undefined) ?? [];
    deleted = ((await run({ type: "vault.query", deleted: true })) as Summary[] | undefined) ?? [];
  }
  $effect(() => {
    void status;
    void load();
    void call({ type: "vault.status" })
      .then((r) => (pending = (r as { pendingImport: string | null }).pendingImport))
      .catch(() => {});
  });

  async function token(): Promise<string | null> {
    const r = (await run({ type: "app.authenticate", reason: t("passwords.reason.reveal") })) as
      { token: string } | undefined;
    return r?.token ?? null;
  }
  async function open(id: string, reveal = false) {
    editing = null;
    detail = (await run({ type: "vault.get", id, token: reveal ? await token() : null })) as Detail;
  }
  async function copy(field: "username" | "password" | "code") {
    if (!detail) return;
    let value = field === "code" ? detail.code : (detail.item[field] as string | undefined);
    if (field === "password") {
      const full = (await run({ type: "vault.get", id: detail.id, token: await token() })) as
        Detail | undefined;
      value = full?.item.password as string | undefined;
    }
    if (value) await navigator.clipboard?.writeText(value).catch(() => {});
  }
  async function startEdit() {
    if (!detail) return;
    const full = (await run({ type: "vault.get", id: detail.id, token: await token() })) as
      Detail | undefined;
    if (!full) return;
    detail = full;
    editing = {
      username: String(full.item.username ?? ""),
      password: String(full.item.password ?? ""),
    };
  }
  async function saveEdit() {
    if (!detail || !editing) return;
    await run({ type: "vault.save", id: detail.id, item: { ...detail.item, ...editing } });
    editing = null;
    await open(detail.id);
    await load();
  }
  async function add() {
    await run({
      type: "vault.save",
      item: {
        kind: "login",
        title: "",
        urls: [adding.url],
        username: adding.username,
        password: adding.password,
        passwordHistory: [],
      },
    });
    adding = { url: "", username: "", password: "" };
    await load();
  }
  async function remove(id: string) {
    await run({ type: "vault.delete", id });
    detail = null;
    await load();
  }
  async function scanQr() {
    const w = Object.values(store.state?.windows ?? {}).find((x) => x.activeTabId);
    if (!w?.activeTabId) return;
    const scan = (await run({ type: "vault.scanQr", tabId: w.activeTabId })) as
      { scanId: string; logins: { id: string }[] } | undefined;
    if (scan)
      await run({
        type: "vault.attachTotp",
        scanId: scan.scanId,
        itemId: scan.logins[0]?.id ?? null,
      });
    await load();
  }
  const setup = async () => {
    const r = (await run({ type: "vault.setup" })) as { recoverySecret: string } | undefined;
    secret = r?.recoverySecret ?? "";
  };
  const exportVault = async () => {
    if (files.passphrase !== files.again) {
      message = t("passwords.export.mismatch");
      return;
    }
    await run({ type: "vault.export", path: files.vaultPath, passphrase: files.passphrase });
  };
</script>

<section data-testid="passwords-section">
  <h2>{t("passwords.title")}</h2>
  {#if message}<p role="alert">{message}</p>{/if}
  {#if status === "none"}
    <p>{t("passwords.off")}</p>
    <button type="button" onclick={setup}>{t("passwords.setup")}</button>
  {:else if status === "locked"}
    <button type="button" onclick={() => run({ type: "vault.unlock" })}
      >{t("passwords.unlock")}</button
    >
    <label
      >{t("passwords.recover")}
      <input bind:value={recoverWith} placeholder="ABCDE-FGHIJ-…" /></label
    >
    <button
      type="button"
      disabled={!recoverWith}
      onclick={() => run({ type: "vault.recover", secret: recoverWith })}
      >{t("passwords.recover.run")}</button
    >
  {:else}
    {#if secret}
      <div role="status">
        <p>{t("passwords.secret")}</p>
        <pre>{secret}</pre>
        <button type="button" onclick={() => (secret = "")}>{t("passwords.secret.saved")}</button>
      </div>
    {/if}
    <button type="button" onclick={() => run({ type: "vault.lock" })}>{t("passwords.lock")}</button>
    {#if pending}
      <p>{t("passwords.pendingImport")}</p>
      <input type="password" bind:value={files.passphrase} aria-label={t("passwords.passphrase")} />
      <button
        type="button"
        onclick={() =>
          run({ type: "vault.import", path: pending, passphrase: files.passphrase }).then(load)}
        >{t("passwords.import.run")}</button
      >
    {/if}

    <input
      type="search"
      bind:value={query}
      aria-label={t("passwords.search")}
      placeholder={t("passwords.search")}
    />
    <ul>
      {#each shown as i (i.id)}
        <li>
          <button type="button" onclick={() => open(i.id)}
            >{i.title}{#if i.username}<small> {i.username}</small>{/if}</button
          >
        </li>
      {/each}
    </ul>

    {#if detail}
      <div aria-label={t("passwords.detail")} role="region">
        <h3>{String(detail.item.title || detail.item.rpId || "")}</h3>
        {#if detail.item.username !== undefined}
          <p>
            {t("passwords.username")}: {String(detail.item.username)}
            <button type="button" onclick={() => copy("username")}>{t("passwords.copy")}</button>
          </p>
        {/if}
        {#if detail.item.kind === "login"}
          <p>
            {t("passwords.password")}: {detail.revealed ? String(detail.item.password) : "••••••••"}
            <button type="button" onclick={() => open(detail?.id ?? "", true)}
              >{t("passwords.reveal")}</button
            >
            <button type="button" onclick={() => copy("password")}>{t("passwords.copy")}</button>
          </p>
        {/if}
        {#if detail.code}
          <p>
            {t("passwords.code")}: {detail.code} ({t("passwords.code.remaining", {
              seconds: detail.codeRemaining ?? 0,
            })})
            <button type="button" onclick={() => copy("code")}>{t("passwords.copy")}</button>
          </p>
        {/if}
        {#if editing}
          <label>{t("passwords.username")} <input bind:value={editing.username} /></label>
          <label>{t("passwords.password")} <input bind:value={editing.password} /></label>
          <button type="button" onclick={saveEdit}>{t("passwords.save")}</button>
        {:else if detail.item.kind === "login"}
          <button type="button" onclick={startEdit}>{t("passwords.edit")}</button>
        {/if}
        <button type="button" onclick={() => remove(detail?.id ?? "")}
          >{t("passwords.delete")}</button
        >
      </div>
    {/if}

    <h3>{t("passwords.add")}</h3>
    <label
      >{t("passwords.site")}
      <input bind:value={adding.url} placeholder="https://example.com" /></label
    >
    <label>{t("passwords.username")} <input bind:value={adding.username} /></label>
    <label>{t("passwords.password")} <input type="password" bind:value={adding.password} /></label>
    <button type="button" disabled={!adding.url || !adding.password} onclick={add}
      >{t("passwords.add.run")}</button
    >
    <button type="button" onclick={scanQr}>{t("passwords.scanQr")}</button>

    <h3>{t("passwords.health")}</h3>
    <button
      type="button"
      onclick={async () => (health = (await run({ type: "vault.health" })) as Health)}
      >{t("passwords.health.run")}</button
    >
    <button
      type="button"
      onclick={async () =>
        (health = (await run({ type: "vault.health", checkLeaks: true })) as Health)}
      >{t("passwords.health.leaks")}</button
    >
    {#if health}
      <p>
        {t("passwords.health.summary", {
          reused: health.reused.length,
          weak: health.weak.length,
          leaked: health.leaked?.length ?? 0,
        })}
      </p>
      <ul>
        {#each health.weak as h (h.id)}<li>
            {t("passwords.health.weak")}: {h.title}
            {h.username}
          </li>{/each}
        {#each health.leaked ?? [] as h (h.id)}<li>
            {t("passwords.health.leaked")}: {h.title}
            {h.username}
          </li>{/each}
        {#each health.reused as group, g (g)}<li>
            {t("passwords.health.reused")}: {group.map((h) => h.title).join(", ")}
          </li>{/each}
      </ul>
    {/if}

    <h3>{t("passwords.deleted")}</h3>
    <ul>
      {#each deleted as d (d.id)}
        <li>
          {d.title}
          <button type="button" onclick={() => run({ type: "vault.restore", id: d.id }).then(load)}
            >{t("passwords.restore")}</button
          >
          <button
            type="button"
            onclick={() => run({ type: "vault.deleteForever", id: d.id }).then(load)}
            >{t("passwords.deleteForever")}</button
          >
        </li>
      {/each}
    </ul>

    <h3>{t("passwords.files")}</h3>
    <label>{t("passwords.csvIn")} <input bind:value={files.csvIn} /></label>
    <button
      type="button"
      disabled={!files.csvIn}
      onclick={() => run({ type: "vault.importCsv", path: files.csvIn }).then(load)}
      >{t("passwords.import.run")}</button
    >
    <label>{t("passwords.csvOut")} <input bind:value={files.csvOut} /></label>
    <button
      type="button"
      disabled={!files.csvOut}
      onclick={() => run({ type: "vault.exportCsv", path: files.csvOut })}
      >{t("passwords.export.run")}</button
    >
    <label>{t("passwords.vaultFile")} <input bind:value={files.vaultPath} /></label>
    <label
      >{t("passwords.passphrase")} <input type="password" bind:value={files.passphrase} /></label
    >
    <label
      >{t("passwords.passphrase.again")} <input type="password" bind:value={files.again} /></label
    >
    <button type="button" disabled={!files.vaultPath} onclick={exportVault}
      >{t("passwords.exportVault")}</button
    >
    <button
      type="button"
      disabled={!files.vaultPath}
      onclick={() =>
        run({ type: "vault.import", path: files.vaultPath, passphrase: files.passphrase }).then(
          load,
        )}>{t("passwords.importVault")}</button
    >
  {/if}
</section>
