<script lang="ts">
  // One generated settings row: the control follows the setting's type.
  import type { Client } from "../core/client";
  import { CoreError } from "../core/client";
  import type { SettingEntry } from "../core/settings-index";
  import Icon from "../design/Icon.svelte";
  import { t } from "../i18n";

  let {
    entry,
    value,
    client,
    controlledBy,
  }: {
    entry: SettingEntry;
    value: unknown;
    client: Client;
    /** The extension whose choice set this (spec 06 §7). */
    controlledBy?: { id: string; name: string };
  } = $props();

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
              ...Object.fromEntries(Object.entries(e.error.args).map(([k, v]) => [k, String(v)])),
              key: t(entry.label),
              reason: String(e.error.args.reason ?? ""),
            })
          : String(e);
    }
  }

  // An option's label: its translation when there is one, else the value
  // in words (`askFirst` reads "Ask first").
  function optionLabel(option: string): string {
    const key = `${entry.label.replace(/\.label$/, "")}.option.${option}`;
    const text = t(key);
    if (text !== key) return text;
    const words = option
      .replace(/([a-z])([A-Z])/g, "$1 $2")
      .replace(/[-_]/g, " ")
      .toLowerCase();
    return words.charAt(0).toUpperCase() + words.slice(1);
  }

  const MINUTE = 60_000;

  const KEYS: Record<string, string> = {
    ArrowLeft: "Left",
    ArrowRight: "Right",
    ArrowUp: "Up",
    ArrowDown: "Down",
    BracketLeft: "[",
    BracketRight: "]",
    Comma: ",",
    Period: ".",
    Equal: "=",
    Minus: "-",
    Slash: "/",
    Backslash: "\\",
    Semicolon: ";",
    Quote: "'",
    Backquote: "`",
    Space: "Space",
    Tab: "Tab",
  };

  /** A pressed key as a shortcut setting, `Cmd+Shift+T`, by key position. */
  function recorded(e: KeyboardEvent): string | null {
    const key = e.code.startsWith("Key")
      ? e.code.slice(3)
      : e.code.startsWith("Digit")
        ? e.code.slice(5)
        : /^F\d+$/.test(e.code)
          ? e.code
          : KEYS[e.code];
    if (!key) return null;
    const mods = [
      e.ctrlKey && "Ctrl",
      e.altKey && "Alt",
      e.shiftKey && "Shift",
      e.metaKey && "Cmd",
    ];
    const held = mods.filter((m): m is string => typeof m === "string");
    return held.length ? [...held, key].join("+") : null;
  }
  const lines = (text: string) =>
    text
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l !== "");
</script>

<div class="row" id={`row-${entry.key}`} data-renderer={entry.renderer}>
  <div class="text">
    <label for={id}
      >{t(entry.label)}{#if entry.protected}<span class="lock" title={t("settings.protected")}
          ><Icon name="lock" size={11} /></span
        >{/if}</label
    >
    <p class="help">{t(entry.help)}</p>
    {#if error}<p role="alert">{error}</p>{/if}
  </div>
  {#if controlledBy}
    <p data-testid="controlled-by">
      {t("settings.controlledBy", { name: controlledBy.name })}
      <button
        type="button"
        onclick={() =>
          client
            .call({ type: "extension.setEnabled", id: controlledBy.id, enabled: false })
            .catch(() => {})}>{t("settings.turnOffExtension")}</button
      >
    </p>
  {/if}
  {#if entry.renderer === "toggle"}
    <input
      {id}
      type="checkbox"
      class="switch"
      checked={value === true}
      onchange={(e) => set(e.currentTarget.checked)}
    />
  {:else if entry.renderer === "select"}
    <select {id} value={String(value)} onchange={(e) => set(e.currentTarget.value)}>
      {#each entry.options ?? [] as option (option)}
        <option value={option}>{optionLabel(option)}</option>
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
  {:else if entry.renderer === "shortcut"}
    <input
      {id}
      type="text"
      readonly
      value={String(value ?? "")}
      placeholder={t("settings.shortcut.press")}
      onkeydown={(e) => {
        if (e.key === "Backspace" || e.key === "Delete") {
          e.preventDefault();
          void set("");
          return;
        }
        const keys = recorded(e);
        if (keys) {
          e.preventDefault();
          void set(keys);
        }
      }}
    />
    <button
      type="button"
      onclick={() => client.call({ type: "settings.reset", key: entry.key }).catch(() => {})}
      >{t("settings.shortcut.reset")}</button
    >
  {:else if entry.renderer === "list" && entry.options}
    <fieldset {id}>
      {#each entry.options as option (option)}
        <label
          ><input
            type="checkbox"
            checked={Array.isArray(value) && value.includes(option)}
            onchange={(e) =>
              set(
                (entry.options ?? []).filter((o) =>
                  o === option
                    ? e.currentTarget.checked
                    : Array.isArray(value) && value.includes(o),
                ),
              )}
          />
          {option}</label
        >
      {/each}
    </fieldset>
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
</div>

<style>
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3) var(--space-6);
    min-height: 48px;
    padding: var(--space-4) var(--space-6);
  }
  .row + :global(.row) {
    border-top: 1px solid var(--line);
  }
  .text {
    flex: 1 1 240px;
    min-width: 0;
  }
  label {
    display: inline-flex;
    gap: var(--space-2);
    font-weight: var(--weight-regular);
  }
  .lock {
    color: var(--text-tertiary);
  }
  .help {
    margin-top: var(--space-1);
    font-size: var(--text-small);
    color: var(--text-secondary);
  }
  .row[data-renderer="list"] textarea,
  .row[data-renderer="json"] textarea {
    flex-basis: 100%;
  }
  [data-testid="controlled-by"] {
    flex-basis: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-control);
    background: var(--warning-tint);
    font-size: var(--text-small);
  }
  fieldset {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-5);
    margin: 0;
    padding: 0;
    border: 0;
  }
  select {
    min-width: 160px;
  }
  input[type="number"] {
    width: 90px;
  }
</style>
