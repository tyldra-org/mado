// The typed command client: one `call` per Command, failing with the
// core's MadoError.
import type { Command } from "../generated/Command";
import type { MadoError } from "../generated/MadoError";
import type { Transport } from "./transport";

export class CoreError extends Error {
  constructor(readonly error: MadoError) {
    super(error.messageKey);
    this.name = "CoreError";
  }
}

let next = 0;

export class Client {
  constructor(private readonly transport: Transport) {}

  async call(cmd: Command): Promise<unknown> {
    next += 1;
    const response = await this.transport.invoke({ id: String(next), cmd });
    if (response.status === "err") throw new CoreError(response.error);
    return response.value;
  }
}
