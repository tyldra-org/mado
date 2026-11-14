// The UI's copy of the state document (mado-docs/spec/01 section 3.1).
// Patches apply strictly in order; a gap or a failed patch drops the copy and
// fetches the whole document again with state.get, once at a time.
import type { Patch } from "../generated/Patch";
import type { State } from "../generated/State";
import type { StateSnapshot } from "../generated/StateSnapshot";
import { applyPatch } from "./patch";

export type FetchState = () => Promise<StateSnapshot>;

export class Mirror {
  state: State | null = null;
  rev = -1;
  #fetching: Promise<void> | null = null;
  /** Patches that arrived while a snapshot was on its way. */
  #queued: Patch[] = [];
  /** How many times the copy was fetched whole; spec 01 §6 check 6 counts it. */
  fetches = 0;

  constructor(
    private readonly fetchState: FetchState,
    private readonly onChange: (state: State) => void,
  ) {}

  /** Fetches the document; call once at start and after a UI reload. */
  sync(): Promise<void> {
    if (this.#fetching) return this.#fetching;
    this.fetches += 1;
    this.#fetching = this.fetchState()
      .then((snapshot) => {
        this.state = snapshot.state;
        this.rev = snapshot.rev;
        const queued = this.#queued.filter((p) => p.rev > snapshot.rev);
        this.#queued = [];
        this.onChange(this.state);
        for (const patch of queued) this.receive(patch);
      })
      .finally(() => {
        this.#fetching = null;
      });
    return this.#fetching;
  }

  receive(patch: Patch): void {
    if (this.#fetching || this.state === null) {
      this.#queued.push(patch);
      return;
    }
    if (patch.rev <= this.rev) return;
    if (patch.rev !== this.rev + 1) {
      this.#queued.push(patch);
      void this.sync();
      return;
    }
    try {
      this.state = applyPatch(this.state, patch.ops);
      this.rev = patch.rev;
      this.state.rev = patch.rev;
      this.onChange(this.state);
    } catch {
      void this.sync();
    }
  }
}
