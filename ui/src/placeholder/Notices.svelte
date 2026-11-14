<script lang="ts">
  // Toasts with Undo (spec 01 §3, 06 §3). Placeholder until M6.
  import type { CoreStore } from "../core/store.svelte";
  import type { Notice } from "../generated/Notice";
  import { t } from "../i18n";

  let { store }: { store: CoreStore } = $props();

  const notices = $derived(store.state?.notices ?? []);

  function message(notice: Notice): string {
    // Arguments named `label` are en.json keys themselves.
    const args = Object.fromEntries(
      Object.entries(notice.args).map(([k, v]) => [k, k === "label" ? t(String(v)) : String(v)]),
    );
    return t(notice.messageKey, args);
  }
</script>

<section aria-label={t("notices.label")} aria-live="polite">
  {#each notices as notice (notice.id)}
    <div role="status">
      <span>{message(notice)}</span>
      {#if notice.action}
        {@const action = notice.action}
        <button type="button" onclick={() => store.client.call(action.command)}>
          {t(action.labelKey)}
        </button>
      {/if}
    </div>
  {/each}
</section>
