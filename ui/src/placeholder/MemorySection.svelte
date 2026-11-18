<script lang="ts">
  // Settings › AI: memory files and skills (spec 13 §10, §11). Placeholder
  // until the design pass (M6): list, read, edit and delete the files,
  // pause memory, file the inbox and undo a day's filing; list and delete
  // skills.
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();

  type File = { path: string; bytes: number };
  type Skill = { name: string; description: string; builtin: boolean };
  const ai = $derived(store.state?.features?.ai === true);
  const on = $derived(store.state?.features?.memory === true);
  let files: File[] = $state([]);
  let skills: Skill[] = $state([]);
  let open = $state("");
  let text = $state("");
  let day = $state("");
  let message = $state("");

  const fail = (e: { error?: { messageKey?: string } }) =>
    (message = t(e.error?.messageKey ?? "error.unknown"));
  const load = () => {
    store.client
      .call({ type: "ai.skill.list" })
      .then((r: unknown) => (skills = r as Skill[]))
      .catch(() => {});
    if (!on) return;
    store.client
      .call({ type: "ai.memory.list" })
      .then((r: unknown) => (files = r as File[]))
      .catch(() => {});
  };
  $effect(() => {
    if (ai) load();
  });
  const show = (path: string) =>
    store.client.call({ type: "ai.memory.read", path }).then((r: unknown) => {
      open = path;
      text = (r as { text: string }).text;
    }, fail);
  const done = () => {
    message = "";
    load();
  };
  const save = () =>
    store.client.call({ type: "ai.memory.write", path: open, text }).then(done, fail);
  const remove = (path: string) =>
    store.client.call({ type: "ai.memory.delete", path }).then(() => {
      if (open === path) open = "";
      done();
    }, fail);
  const pause = () =>
    store.client
      .call({ type: "settings.set", key: "features.memory", value: false, token: null })
      .then(done, fail);
  const fileNow = () => store.client.call({ type: "ai.memory.file" }).then(done, fail);
  const undo = () => store.client.call({ type: "ai.memory.undo", day }).then(done, fail);
  const removeSkill = (name: string) =>
    store.client.call({ type: "ai.skill.delete", name }).then(done, fail);
</script>

{#if ai}
  <section data-testid="ai-memory">
    <h3>{t("ai.memory.title")}</h3>
    {#if message}<p role="alert">{message}</p>{/if}
    {#if !on}
      <p>{t("ai.memory.off")}</p>
    {:else}
      {#if files.length === 0}<p>{t("ai.memory.empty")}</p>{/if}
      <ul>
        {#each files as f (f.path)}
          <li>
            {f.path}
            <button type="button" onclick={() => show(f.path)}>{t("ai.memory.open")}</button>
            <button type="button" onclick={() => remove(f.path)}>{t("ai.memory.delete")}</button>
          </li>
        {/each}
      </ul>
      {#if open}
        <label>{open} <textarea bind:value={text}></textarea></label>
        <button type="button" onclick={save}>{t("ai.memory.save")}</button>
      {/if}
      <button type="button" onclick={fileNow}>{t("ai.memory.fileNow")}</button>
      <label>{t("ai.memory.undoDay")} <input placeholder="YYYY-MM-DD" bind:value={day} /></label>
      <button type="button" disabled={!day} onclick={undo}>{t("ai.memory.undo")}</button>
      <button type="button" onclick={pause}>{t("ai.memory.pause")}</button>
    {/if}
    <h3>{t("ai.skills.title")}</h3>
    <ul>
      {#each skills as s (s.name)}
        <li>
          <strong>{s.name}</strong>
          {s.description}
          {#if s.builtin}<small>{t("ai.skills.builtin")}</small>{:else}
            <button type="button" onclick={() => removeSkill(s.name)}
              >{t("ai.memory.delete")}</button
            >
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}
