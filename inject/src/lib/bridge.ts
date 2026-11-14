// The Mado-world channel the bridge script set up (spec 02 §5.2).

export type Bridge = {
  send(script: string, type: string, payload: unknown): Promise<unknown>;
  on(type: string, listener: (message: Record<string, unknown>) => void): void;
};

/** The bridge, or null in a frame without one. Never throws. */
export function bridge(): Bridge | null {
  const b = (globalThis as Record<symbol, unknown>)[Symbol.for("mado.bridge")];
  return b && typeof b === "object" ? (b as Bridge) : null;
}

/** Sends and ignores failures: the core drops what it does not accept. */
export function sender(script: string) {
  const b = bridge();
  return (type: string, payload: unknown) => {
    b?.send(script, type, payload).catch(() => {});
  };
}
