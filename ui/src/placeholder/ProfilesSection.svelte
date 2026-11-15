<script lang="ts">
  // The Profiles section of settings (spec 12 §1): profiles with rename,
  // colour, open and delete, and the link rules. Placeholder until M6.
  import type { Client } from "../core/client";
  import { t } from "../i18n";

  type Profile = { id: string; name: string; color: string; current: boolean; running: boolean };
  type Rule = { pattern: string; profileId: string };
  const COLORS = ["blue", "red", "orange", "yellow", "green", "teal", "purple", "pink", "grey"];

  let { client }: { client: Client } = $props();
  let profiles = $state<Profile[]>([]);
  let rules = $state<Rule[]>([]);
  let newName = $state("");
  let confirming = $state<string | null>(null);
  let error = $state<string | null>(null);
  let file = $state("");

  async function load() {
    profiles = (await client.call({ type: "profile.list" }).catch(() => [])) as Profile[];
    rules = (await client.call({ type: "profile.linkRules" }).catch(() => [])) as Rule[];
  }
  $effect(() => {
    void load();
  });

  const run = async (cmd: Parameters<Client["call"]>[0]) => {
    error = null;
    await client.call(cmd).catch((e: { error?: { messageKey?: string } }) => {
      error = e?.error?.messageKey ?? "error.internal";
    });
    await load();
  };

  async function saveRules(next: Rule[]) {
    await run({ type: "profile.linkRules.set", rules: next });
  }
</script>

<section aria-labelledby="profiles-title">
  <h2 id="profiles-title">{t("profiles.title")}</h2>
  {#if error}<p role="alert">{t(error)}</p>{/if}
  <ul>
    {#each profiles as p (p.id)}
      <li>
        <input
          aria-label={t("profiles.name")}
          value={p.name}
          onchange={(e) =>
            run({
              type: "profile.update",
              id: p.id,
              name: e.currentTarget.value,
              color: null,
              avatar: null,
            })}
        />
        <select
          aria-label={t("profiles.color")}
          value={p.color}
          onchange={(e) =>
            run({
              type: "profile.update",
              id: p.id,
              name: null,
              color: e.currentTarget.value,
              avatar: null,
            })}
        >
          {#each COLORS as c (c)}<option value={c}>{t(`profiles.colors.${c}`)}</option>{/each}
        </select>
        {#if p.current}
          <span>{t("profiles.current")}</span>
        {:else}
          <button type="button" onclick={() => run({ type: "profile.open", id: p.id })}
            >{t("profiles.open")}</button
          >
          {#if confirming === p.id}
            <p>{t("profiles.deleteWarning", { name: p.name })}</p>
            <button
              type="button"
              onclick={() => ((confirming = null), run({ type: "profile.delete", id: p.id }))}
              >{t("profiles.deleteConfirm", { name: p.name })}</button
            >
            <button type="button" onclick={() => (confirming = null)}>{t("dialog.cancel")}</button>
          {:else}
            <button type="button" onclick={() => (confirming = p.id)}>{t("profiles.delete")}</button
            >
          {/if}
        {/if}
      </li>
    {/each}
  </ul>
  <label>
    {t("profiles.newName")}
    <input bind:value={newName} />
  </label>
  <button
    type="button"
    disabled={!newName.trim()}
    onclick={() =>
      run({ type: "profile.create", name: newName, color: "blue", avatar: "" }).then(
        () => (newName = ""),
      )}>{t("profiles.add")}</button
  >

  <h3>{t("profiles.file")}</h3>
  <label
    >{t("profiles.filePath")}
    <input bind:value={file} placeholder="/Users/you/Desktop/Work.madoprofile" /></label
  >
  {#each profiles.filter((p) => p.current) as p (p.id)}
    <button
      type="button"
      disabled={!file.trim()}
      onclick={() => run({ type: "profile.export", id: p.id, path: file.trim(), passphrase: null })}
      >{t("profiles.export", { name: p.name })}</button
    >
  {/each}
  <button
    type="button"
    disabled={!file.trim()}
    onclick={() => run({ type: "profile.import", path: file.trim(), passphrase: null })}
    >{t("profiles.import")}</button
  >

  <h3>{t("profiles.linkRules")}</h3>
  <p>{t("profiles.linkRulesHelp")}</p>
  <ul>
    {#each rules as r, i (i)}
      <li>
        {r.pattern} → {profiles.find((p) => p.id === r.profileId)?.name ?? r.profileId}
        <button type="button" onclick={() => saveRules(rules.filter((_, j) => j !== i))}
          >{t("profiles.removeRule")}</button
        >
      </li>
    {/each}
  </ul>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      const f = new FormData(e.currentTarget);
      const pattern = String(f.get("pattern") ?? "").trim();
      const profileId = String(f.get("profile") ?? "");
      if (pattern && profileId) void saveRules([...rules, { pattern, profileId }]);
    }}
  >
    <input name="pattern" aria-label={t("profiles.pattern")} placeholder="*.example.com" />
    <select name="profile" aria-label={t("profiles.ruleProfile")}>
      {#each profiles as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
    </select>
    <button type="submit">{t("profiles.addRule")}</button>
  </form>
</section>
