// `context` (spec 02 §5, 07 §8): what is under the pointer when the context
// menu opens. Mado world, document start, all frames.
import { sender } from "./lib/bridge";

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
