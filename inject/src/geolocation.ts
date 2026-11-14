// `geolocation` (spec 07 §1.1): replaces `navigator.geolocation` before
// macOS 27, where WebKit has no permission delegate for it. Page world,
// document start, main frame. Injected only on those versions.

const ask = (type: string, payload: unknown) =>
  document.dispatchEvent(new CustomEvent("mado:request", { detail: { type, payload } }));

type Waiter = { ok: PositionCallback; err?: PositionErrorCallback | null; watch: boolean };
const waiting = new Map<number, Waiter>();
let next = 1;

document.addEventListener("mado:reply", (e) => {
  let m: {
    type?: string;
    id?: number;
    coords?: GeolocationCoordinates;
    timestamp?: number;
    error?: number;
  };
  try {
    m = JSON.parse((e as CustomEvent).detail);
  } catch {
    return;
  }
  if (m.type !== "geolocationPosition" || typeof m.id !== "number") return;
  const w = waiting.get(m.id);
  if (!w) return;
  if (!w.watch) waiting.delete(m.id);
  if (m.coords)
    w.ok({ coords: m.coords, timestamp: m.timestamp ?? Date.now() } as GeolocationPosition);
  else
    w.err?.({
      code: m.error ?? 1,
      message: "",
      PERMISSION_DENIED: 1,
      POSITION_UNAVAILABLE: 2,
      TIMEOUT: 3,
    } as GeolocationPositionError);
});

const start = (
  watch: boolean,
  ok: PositionCallback,
  err?: PositionErrorCallback | null,
  options?: PositionOptions,
) => {
  const id = next++;
  waiting.set(id, { ok, err, watch });
  ask(watch ? "geolocationWatch" : "geolocationGet", {
    id,
    highAccuracy: options?.enableHighAccuracy === true,
  });
  return id;
};

Object.defineProperty(navigator, "geolocation", {
  value: Object.freeze({
    getCurrentPosition: (
      ok: PositionCallback,
      err?: PositionErrorCallback | null,
      o?: PositionOptions,
    ) => {
      start(false, ok, err, o);
    },
    watchPosition: (
      ok: PositionCallback,
      err?: PositionErrorCallback | null,
      o?: PositionOptions,
    ) => start(true, ok, err, o),
    clearWatch: (id: number) => {
      waiting.delete(id);
      ask("geolocationClear", { id });
    },
  }),
  configurable: true,
});

export {};
