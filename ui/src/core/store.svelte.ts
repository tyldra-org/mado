// The reactive state store every component reads: the mirror of the core's
// document plus the command client.
import type { Patch } from "../generated/Patch";
import type { State } from "../generated/State";
import type { StateSnapshot } from "../generated/StateSnapshot";
import { Client } from "./client";
import { Mirror } from "./mirror";
import type { Transport } from "./transport";

export class CoreStore {
  state = $state.raw<State | null>(null);
  readonly client: Client;
  readonly mirror: Mirror;

  constructor(readonly transport: Transport) {
    this.client = new Client(transport);
    this.mirror = new Mirror(
      () => this.client.call({ type: "state.get" }) as Promise<StateSnapshot>,
      (state) => {
        this.state = state;
      },
    );
  }

  /** Subscribes to patches, then fetches the document. */
  async start(): Promise<void> {
    await this.transport.listen<Patch>("mado://patch", (patch) => this.mirror.receive(patch));
    await this.mirror.sync();
  }
}
