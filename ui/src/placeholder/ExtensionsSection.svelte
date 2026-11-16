<script lang="ts">
  // Settings › Extensions (spec 09 §11): every management command, and in
  // Developer mode installs from a folder or file, errors and inspection.
  // Placeholder until the design pass (M6).
  import type { Command } from "../generated/Command";
  import type { CoreStore } from "../core/store.svelte";
  import type { ExtensionSource } from "../generated/ExtensionSource";
  import type { InstalledExtension } from "../generated/InstalledExtension";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();
  const installed = $derived(store.state?.extensions?.installed ?? []);
  const pinned = $derived(new Set(store.state?.extensions?.pinned ?? []));
  const developer = $derived(store.state?.settings.values["extensions.developerMode"] === true);

  let message = $state("");
  type Overlap = { category: string; choice: string; name: string | null };
  let overlaps: Overlap[] = $state([]);
  const loadOverlaps = () =>
    store.client
      .call({ type: "extension.overlaps" })
      .then((r: unknown) => (overlaps = r as Overlap[]))
      .catch(() => {});
  $effect(() => {
    void store.state?.extensions?.controlled;
    void loadOverlaps();
  });
  let errors: Record<string, string[]> = $state({});
  let path = $state("");
  let source: ExtensionSource = $state("unpacked");

  const run = (cmd: Command) =>
    store.client.call(cmd).then(
      () => (message = ""),
      (e: { messageKey?: string; args?: Record<string, string | number> }) =>
        (message = t(e.messageKey ?? "error.unknown", e.args ?? {})),
    );

  const granted = (e: InstalledExtension, p: string) =>
    e.permissions.includes(p) || e.hosts.includes(p);
</script>

<section data-testid="extensions-section">
  <h2>{t("settings.section.extensions")}</h2>
  {#if message}<p role="alert">{message}</p>{/if}
  {#if installed.length === 0}<p>{t("extensions.none")}</p>{/if}
  <ul>
    {#each installed as e (e.id)}
      <li data-extension={e.id}>
        <h3>{e.name} <small>{e.version}</small></h3>
        {#if e.disabledReason}<p>{t(`extensions.disabled.${e.disabledReason}`)}</p>{/if}
        <label
          ><input
            type="checkbox"
            checked={e.enabled}
            onchange={(ev) =>
              run({ type: "extension.setEnabled", id: e.id, enabled: ev.currentTarget.checked })}
          />{t("extensions.enabled")}</label
        >
        <label
          ><input
            type="checkbox"
            checked={pinned.has(e.id)}
            onchange={(ev) =>
              run({ type: "extension.pin", id: e.id, pinned: ev.currentTarget.checked })}
          />{t("extensions.pin")}</label
        >
        <label
          >{t("extensions.siteAccess")}
          <select
            value={e.siteAccess}
            onchange={(ev) =>
              run({
                type: "extension.setSiteAccess",
                id: e.id,
                mode: ev.currentTarget.value as InstalledExtension["siteAccess"],
                hosts: null,
              })}
          >
            <option value="onClick">{t("extensions.siteAccess.onClick")}</option>
            <option value="specific">{t("extensions.siteAccess.specific")}</option>
            <option value="all">{t("extensions.siteAccess.all")}</option>
          </select></label
        >
        <label
          ><input
            type="checkbox"
            checked={e.privateAllowed}
            onchange={(ev) =>
              run({
                type: "extension.setPrivateAllowed",
                id: e.id,
                allowed: ev.currentTarget.checked,
              })}
          />{t("extensions.private")}</label
        >
        {#each e.optional as p (p)}
          <label
            ><input
              type="checkbox"
              checked={granted(e, p)}
              onchange={(ev) =>
                run({
                  type: ev.currentTarget.checked
                    ? "extension.grantOptional"
                    : "extension.revokeOptional",
                  id: e.id,
                  permissions: [p],
                })}
            />{p}</label
          >
        {/each}
        {#each e.shortcuts as s (s.command)}
          <label
            >{s.description || s.command}
            <input
              value={s.keys ?? ""}
              placeholder="Cmd+Shift+Y"
              onchange={(ev) =>
                run({
                  type: "extension.setShortcut",
                  id: e.id,
                  command: s.command,
                  keys: ev.currentTarget.value || null,
                })}
            /></label
          >
        {/each}
        {#if e.hasOptions}
          <button type="button" onclick={() => run({ type: "extension.openOptions", id: e.id })}
            >{t("extensions.options")}</button
          >
        {/if}
        <button
          type="button"
          onclick={() => run({ type: "extension.remove", id: e.id, deleteData: false })}
          >{t("extensions.remove")}</button
        >
        <button
          type="button"
          onclick={() => run({ type: "extension.remove", id: e.id, deleteData: true })}
          >{t("extensions.removeWithData")}</button
        >
        {#if developer}
          {#if e.source === "unpacked" || e.source === "zip"}
            <button type="button" onclick={() => run({ type: "extension.reload", id: e.id })}
              >{t("extensions.reload")}</button
            >
          {/if}
          <button
            type="button"
            onclick={() =>
              store.client
                .call({ type: "extension.errors", id: e.id })
                .then((r: unknown) => (errors[e.id] = r as string[]))
                .catch(() => {})}>{t("extensions.errors")}</button
          >
          <button
            type="button"
            onclick={() => run({ type: "extension.inspect", id: e.id, target: "background" })}
            >{t("extensions.inspect")}</button
          >
          {#if errors[e.id]}
            <pre>{errors[e.id]?.join("\n") || t("extensions.noErrors")}</pre>
          {/if}
        {/if}
      </li>
    {/each}
  </ul>
  {#if overlaps.length > 0}
    <h3>{t("extensions.overlaps")}</h3>
    <ul>
      {#each overlaps as o (o.category)}
        <li>
          {t(`prompt.overlap.${o.category}`, { name: o.name ?? "" })}:
          {t(`prompt.overlap.choice.${o.choice}`, { name: o.name ?? "" })}
          <button
            type="button"
            onclick={() =>
              run({ type: "extension.overlapReset", category: o.category }).then(loadOverlaps)}
            >{t("extensions.overlaps.change")}</button
          >
        </li>
      {/each}
    </ul>
  {/if}
  {#if developer}
    <h3>{t("extensions.load")}</h3>
    <label
      >{t("extensions.load.source")}
      <select bind:value={source}>
        <option value="unpacked">{t("extensions.load.folder")}</option>
        <option value="crxFile">.crx</option>
        <option value="zip">.zip</option>
      </select></label
    >
    <input
      bind:value={path}
      placeholder="/Users/me/my-extension"
      aria-label={t("extensions.load.path")}
    />
    <button
      type="button"
      disabled={path === ""}
      onclick={() => run({ type: "extension.install", id: null, source, path })}
      >{t("extensions.load.install")}</button
    >
  {/if}
</section>
