// `media` (spec 02 §4.4, §5; 05 §11): mute, the media button's data, play
// and pause, and DRM failure detection. Mado world, document start, all frames.
import { bridge, sender } from "./lib/bridge";

const send = sender("media");
let muted = false;
const elements = () => Array.from(document.querySelectorAll<HTMLMediaElement>("video, audio"));

function report() {
  const playing = elements().some((m) => !m.paused && !m.ended);
  const meta = (navigator as Navigator & { mediaSession?: MediaSession }).mediaSession?.metadata;
  send("mediaState", {
    playing,
    muted,
    title: meta?.title?.slice(0, 512) ?? null,
    artist: meta?.artist?.slice(0, 512) ?? null,
  });
}

const apply = (m: HTMLMediaElement) => {
  if (muted) m.muted = true;
};

for (const type of ["play", "pause", "ended", "emptied"]) {
  document.addEventListener(
    type,
    (e) => {
      if (e.target instanceof HTMLMediaElement) {
        if (type === "play") apply(e.target);
        report();
      }
    },
    { capture: true, passive: true },
  );
}

// DRM: an encrypted stream that never gets a key within 8 seconds failed
// (spec 05 §11): the core offers to open the site in Safari.
const watched = new WeakSet<HTMLMediaElement>();
document.addEventListener(
  "encrypted",
  (e) => {
    const m = e.target;
    if (!(m instanceof HTMLMediaElement) || watched.has(m)) return;
    watched.add(m);
    setTimeout(() => {
      if (!m.mediaKeys || (m.readyState < 3 && !m.paused)) send("drmFailed", { keySystem: null });
    }, 8000);
  },
  { capture: true, passive: true },
);

const b = bridge();
b?.on("mute", (m) => {
  muted = m.muted === true;
  for (const el of elements()) el.muted = muted;
  report();
});
b?.on("media", (m) => {
  const all = elements();
  const playing = all.find((el) => !el.paused);
  if (m.action === "playPause") {
    if (playing) playing.pause();
    else all[0]?.play().catch(() => {});
  }
  // "next" and "previous" belong to the page's own media session handlers,
  // which another world cannot call; the media keys reach the page directly.
});
