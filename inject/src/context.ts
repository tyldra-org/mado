// `context` (spec 02 §5, 07 §8): what is under the pointer when the context
// menu opens. Mado world, document start, all frames.
import { bridge, sender } from "./lib/bridge";

const send = sender("context");
const LIMIT = 4096;
const cut = (s: string | null | undefined) => (s ? s.slice(0, LIMIT) : null);

document.addEventListener(
  "contextmenu",
  (e) => {
    const el = e.target instanceof Element ? e.target : null;
    const link = el?.closest("a[href]");
    const media = el?.closest("video, audio");
    const image = el?.closest("img");
    const editable = el?.closest("input, textarea, [contenteditable=''], [contenteditable='true']");
    remember(editable ?? null);
    send("contextTarget", {
      link: link instanceof HTMLAnchorElement ? cut(link.href) : null,
      linkText: link ? cut(link.textContent?.trim()) : null,
      image: image instanceof HTMLImageElement ? cut(image.currentSrc || image.src) : null,
      media: media instanceof HTMLMediaElement ? cut(media.currentSrc || media.src) : null,
      mediaKind:
        media instanceof HTMLVideoElement
          ? "video"
          : media instanceof HTMLAudioElement
            ? "audio"
            : null,
      selection: cut(getSelection()?.toString() || null),
      editable: !!editable,
    });
  },
  { capture: true, passive: true },
);

// Page commands (spec 13 §6): "Replace" and "Insert below" put the
// model's answer where the selection was in the field the menu opened on.
// Text fields take it through the native value setter with `input` and
// `change`, as the forms script fills; editable elements through
// `insertText`, so the page's undo and listeners see it.
type Spot =
  | { kind: "field"; el: HTMLInputElement | HTMLTextAreaElement; start: number; end: number }
  | { kind: "editable"; el: HTMLElement; range: Range };
let spot: Spot | null = null;

function remember(el: Element | null) {
  spot = null;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    spot = {
      kind: "field",
      el,
      start: el.selectionStart ?? 0,
      end: el.selectionEnd ?? el.value.length,
    };
  } else if (el instanceof HTMLElement) {
    const sel = getSelection();
    if (sel && sel.rangeCount > 0)
      spot = { kind: "editable", el, range: sel.getRangeAt(0).cloneRange() };
  }
}

function setValue(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto =
    el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")?.set?.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
  el.dispatchEvent(new Event("change", { bubbles: true }));
}

bridge()?.on("aiReplace", (m) => {
  const text = typeof m.text === "string" ? m.text : "";
  const below = m.mode === "insert";
  const s = spot;
  if (!s || !s.el.isConnected || !text) return;
  if (s.kind === "field") {
    const v = s.el.value;
    const next = below
      ? `${v.slice(0, s.end)}\n${text}${v.slice(s.end)}`
      : `${v.slice(0, s.start)}${text}${v.slice(s.end)}`;
    setValue(s.el, next);
  } else {
    s.el.focus();
    const sel = getSelection();
    if (!sel) return;
    sel.removeAllRanges();
    const r = s.range.cloneRange();
    if (below) r.collapse(false);
    sel.addRange(r);
    document.execCommand("insertText", false, below ? `\n${text}` : text);
  }
  spot = null;
});
