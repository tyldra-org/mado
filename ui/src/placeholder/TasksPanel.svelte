<script lang="ts">
  // The task list and task view (spec 13 §7): each task's status and step,
  // with Stop, an answer when it waits for the person, and Raise limit when
  // a spend limit paused it. Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import Icon from "../design/Icon.svelte";
  import { portal } from "../design/portal";
  import { t } from "../i18n";

  let { store, open = true }: { store: CoreStore; open?: boolean } = $props();
  // The toolbar button shows and hides the panel; it starts as the window asks.
  let shown = $state((() => open)());
  const tasks = $derived(Object.values(store.state?.ai?.tasks ?? {}));
  let answers: Record<string, string> = $state({});
  let prompt = $state("");
  const call = (cmd: Parameters<typeof store.client.call>[0]) =>
    store.client.call(cmd).catch(() => {});
  const start = () =>
    call({
      type: "ai.task.start",
      prompt,
      mode: null,
      access: null,
      runtime: null,
      cli: null,
      model: null,
      effort: null,
      scope: null,
    }).then(() => (prompt = ""));
</script>

{#if store.state?.features?.agents}
  <button
    type="button"
    class="icon"
    aria-expanded={shown}
    aria-label={t("ai.tasks")}
    title={t("ai.tasks")}
    data-running={tasks.some((x) => x.status === "running")}
    onclick={() => (shown = !shown)}><Icon name="tasks" /></button
  >
{/if}
{#if store.state?.features?.agents && shown}
  <section class="panel" aria-label={t("ai.tasks")} {@attach portal("side-panels")}>
    <h3>{t("ai.tasks")}</h3>
    <label>{t("ai.task.new")} <input bind:value={prompt} /></label>
    <button type="button" disabled={!prompt.trim()} onclick={start}>{t("ai.task.start")}</button>
    <ul>
      {#each tasks as task (task.id)}
        <li>
          <strong>{task.title}</strong> · {t(`ai.task.status.${task.status}`)}
          {#if task.step}· {task.step}{/if}
          {#if task.status === "running" || task.status === "waiting" || task.status === "queued"}
            <button type="button" onclick={() => call({ type: "ai.task.stop", taskId: task.id })}
              >{t("ai.task.stop")}</button
            >
          {/if}
          {#if task.status === "waiting"}
            <input aria-label={t("ai.task.answer")} bind:value={answers[task.id]} />
            <button
              type="button"
              onclick={() =>
                call({ type: "ai.task.answer", taskId: task.id, text: answers[task.id] ?? "" })}
              >{t("ai.task.answer")}</button
            >
          {/if}
          {#if task.status === "paused"}
            <button
              type="button"
              onclick={() => call({ type: "ai.task.raiseLimit", taskId: task.id })}
              >{t("ai.task.raiseLimit")}</button
            >
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}
