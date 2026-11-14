// Launches the real app and talks to it through its control endpoint with
// the test token (mado-docs/spec/16-testing.md section 1). The binary must
// be built with the test-hooks feature:
//
//   cargo build -p mado-app --features test-hooks
//   MADO_E2E_BIN=target/debug/mado-app bun test tests/e2e
//
// Each Mado gets its own data folder, runs --hidden, and keeps any window it
// opens hidden and unfocused (MADO_TEST_HEADLESS).

import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Subprocess, spawn } from "bun";

export const bin = process.env.MADO_E2E_BIN;

export class RpcError extends Error {
  constructor(
    readonly code: number,
    message: string,
    readonly data: unknown,
  ) {
    super(message);
  }
}

/** One JSON-RPC connection to a running Mado. */
export class Control {
  #next = 0;
  #buffer = "";
  #pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  #socket: Awaited<ReturnType<typeof Bun.connect>> | null = null;

  async open(path: string): Promise<void> {
    this.#socket = await Bun.connect({
      unix: path,
      socket: {
        data: (_s, chunk) => {
          this.#buffer += chunk.toString();
          for (let i = this.#buffer.indexOf("\n"); i >= 0; i = this.#buffer.indexOf("\n")) {
            const line = this.#buffer.slice(0, i);
            this.#buffer = this.#buffer.slice(i + 1);
            const reply = JSON.parse(line) as {
              id: number;
              result?: unknown;
              error?: { code: number; message: string; data?: unknown };
            };
            const waiter = this.#pending.get(reply.id);
            this.#pending.delete(reply.id);
            if (reply.error)
              waiter?.reject(new RpcError(reply.error.code, reply.error.message, reply.error.data));
            else waiter?.resolve(reply.result);
          }
        },
      },
    });
  }

  call<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    this.#next += 1;
    const id = this.#next;
    const done = new Promise<T>((resolve, reject) => {
      this.#pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
    });
    this.#socket?.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    return done;
  }

  close(): void {
    this.#socket?.end();
  }
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** A running Mado with its own data folder. */
export class Mado {
  process: Subprocess | null = null;
  control = new Control();
  readonly token = crypto.randomUUID().replaceAll("-", "");

  private constructor(readonly dir: string) {}

  static async create(): Promise<Mado> {
    return new Mado(await mkdtemp(join(tmpdir(), "mado-e2e-")));
  }

  env(): Record<string, string> {
    return {
      ...process.env,
      MADO_DATA_DIR: this.dir,
      MADO_TEST_TOKEN: this.token,
      MADO_TEST_HEADLESS: "1",
    } as Record<string, string>;
  }

  /** Runs one more launch of the same Mado and returns its exit code. */
  async run(...args: string[]): Promise<number> {
    if (!bin) throw new Error("MADO_E2E_BIN is not set");
    const p = spawn([bin, ...args], { env: this.env(), stdout: "ignore", stderr: "inherit" });
    return p.exited;
  }

  /** Starts the owner and connects once its control entry appears. */
  async start(...args: string[]): Promise<void> {
    if (!bin) throw new Error("MADO_E2E_BIN is not set");
    this.process = spawn([bin, "--hidden", ...args], {
      env: this.env(),
      stdout: "ignore",
      stderr: "inherit",
    });
    const entries = join(this.dir, "Control");
    for (let i = 0; i < 200; i++) {
      const names = await readdir(entries).catch(() => [] as string[]);
      const name = names.find((n) => n.endsWith(".json"));
      if (name) {
        const entry = JSON.parse(await readFile(join(entries, name), "utf8")) as {
          endpoint: string;
          pid: number;
        };
        if (entry.pid === this.process.pid) {
          this.control = new Control();
          await this.control.open(entry.endpoint);
          await this.control.call("hello", {
            client: { name: "e2e", version: "0" },
            token: this.token,
          });
          return;
        }
      }
      await sleep(50);
    }
    throw new Error("Mado did not start its control endpoint within 10 seconds");
  }

  /** Quits like SIGTERM from the OS. */
  async quit(): Promise<number | null> {
    this.control.close();
    this.process?.kill("SIGTERM");
    return (await this.process?.exited) ?? null;
  }

  kill(): Promise<number> | undefined {
    this.control.close();
    this.process?.kill("SIGKILL");
    return this.process?.exited;
  }

  async remove(): Promise<void> {
    if (this.process && this.process.exitCode === null) await this.kill();
    await rm(this.dir, { recursive: true, force: true });
  }
}
