<script lang="ts">
  // The side chat (spec 13 §5): ask about the current tab. Answers stream
  // on the `mado://ai` event. Placeholder until the design pass (M6).
  import type { CoreStore } from "../core/store.svelte";
  import type { WindowId } from "../generated/WindowId";
  import { t } from "../i18n";

  let { store, windowId }: { store: CoreStore; windowId: WindowId } = $props();

  type Source = { url: string; title: string };
  type Note = { kind: string; url?: string; label?: string };
  type Entry = {
    role: "user" | "assistant";
    text: string;
    sources: Source[];
    notes: Note[];
    error?: string;
  };

  const on = $derived(store.state?.features?.ai === true && !store.state?.profile?.isPrivate);
  const tabId = $derived(store.state?.windows[windowId]?.activeTabId ?? null);
  let open = $state(false);
  let chatId: string | null = $state(null);
  let messageId: string | null = $state(null);
  let entries: Entry[] = $state([]);
  let input = $state("");
  let withTab = $state(true);
  let withScreenshot = $state(false);
  let busy = $state(false);
  let message = $state("");

  type AiEvent = {
    chatId: string;
    messageId?: string;
    delta?: string;
    note?: Note;
    done?: boolean;
    text?: string;
    sources?: Source[];
    notes?: Note[];
    error?: string | null;
  };
  $effect(() => {
    let stop: (() => void) | undefined;
    void store.transport
      .listen<AiEvent>("mado://ai", (e) => {
        if (e.chatId !== chatId) return;
        const last = entries[entries.length - 1];
        if (!last || last.role !== "assistant") return;
        if (e.delta) last.text += e.delta;
        if (e.note) last.notes.push(e.note);
        if (e.done) {
          busy = false;
          if (e.text !== undefined && e.text !== "") last.text = e.text;
          last.sources = e.sources ?? [];
          if (e.error) last.error = e.error;
        }
      })
      .then((u) => (stop = u));
    return () => stop?.();
  });

  async function send() {
    const text = input.trim();
    if (!text || busy) return;
    message = "";
    try {
      const r = (await store.client.call({
        type: "ai.chat.send",
        chatId,
        text,
        context: {
          tabIds: withTab && tabId ? [tabId] : [],
          selection: null,
          screenshotTabId: withScreenshot && tabId ? tabId : null,
          files: [],
        },
      })) as { chatId: string; messageId: string };
      chatId = r.chatId;
      messageId = r.messageId;
      entries.push({ role: "user", text, sources: [], notes: [] });
      entries.push({ role: "assistant", text: "", sources: [], notes: [] });
      input = "";
      busy = true;
    } catch (e) {
      message = t((e as { error?: { messageKey?: string } }).error?.messageKey ?? "error.unknown");
    }
  }
  const stopAnswer = () =>
    chatId && store.client.call({ type: "ai.chat.cancel", chatId }).catch(() => {});
  const fresh = () => {
    chatId = null;
    messageId = null;
    entries = [];
  };
  const noteText = (n: Note) =>
    t(`ai.chat.note.${n.kind}`, { url: n.url ?? "", label: n.label ?? "" });
</script>

{#if on}
  <button type="button" aria-expanded={open} onclick={() => (open = !open)}
    >{t("ai.chat.toggle")}</button
  >
  {#if open}
    <aside aria-label={t("ai.chat.title")} data-message={messageId}>
      <button type="button" onclick={fresh}>{t("ai.chat.new")}</button>
      <ol>
        {#each entries as e, i (i)}
          <li class={e.role}>
            {#each e.notes as n, j (j)}<p class="note">{noteText(n)}</p>{/each}
            <p>{e.text}</p>
            {#if e.error}<p role="alert">{t(`error.ai.${e.error}`)}</p>{/if}
            {#if e.sources.length}
              <ol class="sources">
                {#each e.sources as s, k (k)}<li><a href={s.url}>[{k + 1}] {s.title}</a></li>{/each}
              </ol>
            {/if}
          </li>
        {/each}
      </ol>
      {#if message}<p role="alert">{message}</p>{/if}
      <label><input type="checkbox" bind:checked={withTab} /> {t("ai.chat.withTab")}</label>
      <label
        ><input type="checkbox" bind:checked={withScreenshot} />
        {t("ai.chat.withScreenshot")}</label
      >
      <textarea aria-label={t("ai.chat.input")} bind:value={input}></textarea>
      {#if busy}
        <button type="button" onclick={stopAnswer}>{t("ai.chat.stop")}</button>
      {:else}
        <button type="button" disabled={!input.trim()} onclick={send}>{t("ai.chat.send")}</button>
      {/if}
    </aside>
  {/if}
{/if}
