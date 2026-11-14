<script lang="ts">
  // One generated settings row: the control follows the setting's type.
  import type { Client } from "../core/client";
  import { CoreError } from "../core/client";
  import type { SettingEntry } from "../core/settings-index";
  import { t } from "../i18n";

  let { entry, value, client }: { entry: SettingEntry; value: unknown; client: Client } = $props();

  const id = $props.id();
  let error = $state("");

  async function set(next: unknown) {
    error = "";
    try {
      await client.call({ type: "settings.set", key: entry.key, value: next, token: null });
    } catch (e) {
      error =
        e instanceof CoreError
          ? t(e.error.messageKey, {
              key: t(entry.label),
              reason: String(e.error.args.reason ?? ""),
            })
          : String(e);
    }
  }

  const MINUTE = 60_000;
  const lines = (text: string) =>
    text
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l !== "");
</script>

<div class="row" id={`row-${entry.key}`}>
  <label for={id}>{t(entry.label)}</label>
  {#if entry.renderer === "toggle"}
    <input
      {id}
      type="checkbox"
      checked={value === true}
      onchange={(e) => set(e.currentTarget.checked)}
    />
  {:else if entry.renderer === "select"}
    <select {id} value={String(value)} onchange={(e) => set(e.currentTarget.value)}>
      {#each entry.options ?? [] as option (option)}
        <option value={option}>{option}</option>
      {/each}
    </select>
  {:else if entry.renderer === "number"}
    <input
      {id}
      type="number"
      value={Number(value)}
      onchange={(e) => set(e.currentTarget.valueAsNumber)}
    />
  {:else if entry.renderer === "duration"}
    <input
      {id}
      type="number"
      min="0"
      value={Number(value) / MINUTE}
      onchange={(e) => set(Math.round(e.currentTarget.valueAsNumber * MINUTE))}
    />
    <span>{t("settings.minutes")}</span>
  {:else if entry.renderer === "list"}
    <textarea
      {id}
      rows="3"
      value={Array.isArray(value) ? value.join("\n") : ""}
      onchange={(e) => set(lines(e.currentTarget.value))}></textarea>
  {:else if entry.renderer === "json"}
    <textarea
      {id}
      rows="2"
      value={JSON.stringify(value)}
      onchange={(e) => {
        try {
          void set(JSON.parse(e.currentTarget.value));
        } catch {
          error = t("settings.invalidJson");
        }
      }}></textarea>
  {:else}
    <input
      {id}
      type="text"
      value={String(value ?? "")}
      onchange={(e) => set(e.currentTarget.value)}
    />
  {/if}
  <p class="help">{t(entry.help)}</p>
  {#if error}<p role="alert">{error}</p>{/if}
</div>
