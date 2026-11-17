<script lang="ts">
  // Settings › AI › Models: model pickers and role rows (spec 13 §4).
  // Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();

  type Model = {
    connectionId: string;
    modelId: string;
    name: string;
    show: boolean;
    starred: boolean;
    info: { pricePerMTok: { input: number; output: number } } | null;
  };
  const roleNames = ["chat", "tasks", "fast", "deep", "vision", "commands", "image"];
  const efforts = ["auto", "low", "medium", "high", "xhigh", "max"];

  const on = $derived(store.state?.features?.ai === true);
  const roles = $derived(store.state?.ai?.roles ?? {});
  let models: Model[] = $state([]);
  let message = $state("");
  const ref = (m: Model) => `${m.connectionId}:${m.modelId}`;
  const nameOf = (r: string | null | undefined) =>
    models.find((m) => ref(m) === r)?.name ?? r ?? t("ai.role.none");

  const load = () =>
    store.client
      .call({ type: "ai.model.list" })
      .then((r: unknown) => (models = r as Model[]))
      .catch(() => {});
  $effect(() => {
    void store.state?.ai?.connections;
    if (on) void load();
  });

  const fail = (e: { error?: { messageKey?: string } }) =>
    (message = t(e.error?.messageKey ?? "error.unknown"));
  const setRole = (
    role: string,
    change: { model?: string; effort?: string; fallback?: string },
  ) => {
    const r = roles[role];
    void store.client
      .call({
        type: "ai.role.set",
        role,
        model: change.model ?? r?.model ?? "auto",
        effort: change.effort ?? r?.effort ?? "auto",
        fallback: change.fallback ?? r?.fallback ?? null,
      })
      .then(() => (message = ""), fail);
  };
  const mark = (m: Model, change: { show?: boolean; starred?: boolean }) =>
    store.client
      .call({
        type: "ai.model.set",
        connectionId: m.connectionId,
        modelId: m.modelId,
        show: change.show ?? null,
        starred: change.starred ?? null,
      })
      .then(load, fail);
  const price = (m: Model) =>
    m.info
      ? t("ai.model.price", {
          input: m.info.pricePerMTok.input,
          output: m.info.pricePerMTok.output,
        })
      : t("ai.model.unknownPrice");
</script>

{#if on}
  <section data-testid="ai-models">
    <h3>{t("ai.roles")}</h3>
    {#if message}<p role="alert">{message}</p>{/if}
    {#each roleNames as role (role)}
      <div>
        <strong>{t(`ai.role.${role}`)}</strong>
        <label
          >{t("ai.role.model")}
          <select
            value={roles[role]?.model ?? "auto"}
            onchange={(e) => setRole(role, { model: e.currentTarget.value })}
          >
            <option value="auto"
              >{t("ai.role.auto", { name: nameOf(roles[role]?.resolved) })}</option
            >
            {#each models.filter((m) => m.show) as m (ref(m))}
              <option value={ref(m)}>{m.name}</option>
            {/each}
          </select></label
        >
        <label
          >{t("ai.role.effort")}
          <select
            value={roles[role]?.effort ?? "auto"}
            onchange={(e) => setRole(role, { effort: e.currentTarget.value })}
          >
            {#each efforts as e (e)}
              <option value={e}>{t(`ai.effort.${e}`)}</option>
            {/each}
          </select></label
        >
        <label
          >{t("ai.role.fallback")}
          <select
            value={roles[role]?.fallback ?? ""}
            onchange={(e) => setRole(role, { fallback: e.currentTarget.value })}
          >
            <option value="">{t("ai.role.noFallback")}</option>
            {#each models.filter((m) => m.show) as m (ref(m))}
              <option value={ref(m)}>{m.name}</option>
            {/each}
          </select></label
        >
      </div>
    {/each}
    <h3>{t("ai.models")}</h3>
    <ul>
      {#each models as m (ref(m))}
        <li>
          {m.name} · {price(m)}
          <label
            ><input
              type="checkbox"
              checked={m.show}
              onchange={(e) => mark(m, { show: e.currentTarget.checked })}
            />
            {t("ai.model.show")}</label
          >
          <label
            ><input
              type="checkbox"
              checked={m.starred}
              onchange={(e) => mark(m, { starred: e.currentTarget.checked })}
            />
            {t("ai.model.star")}</label
          >
        </li>
      {/each}
    </ul>
  </section>
{/if}
