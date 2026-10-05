// JSON-RPC 2.0 over newline-delimited JSON, matching `kara serve --stdio`.
// No VS Code imports: this module is unit-tested with node:test.

import { EventEmitter } from "events";

export interface RpcErrorObject {
  code: number;
  message: string;
  data?: unknown;
}

export class RpcError extends Error {
  constructor(public readonly code: number, message: string, public readonly data?: unknown) {
    super(message);
  }
}

type Pending = { resolve: (v: unknown) => void; reject: (e: Error) => void };

export type RequestHandler = (method: string, params: unknown) => Promise<unknown>;

/**
 * Bidirectional JSON-RPC peer. Feed incoming bytes to `receive`; outgoing
 * lines go to `write`. Notifications are emitted as `notification` events.
 */
export class RpcClient extends EventEmitter {
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private buffer = "";
  private requestHandler: RequestHandler | undefined;
  private closed = false;

  constructor(private readonly write: (line: string) => void) {
    super();
  }

  onRequest(handler: RequestHandler): void {
    this.requestHandler = handler;
  }

  request<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    if (this.closed) {
      return Promise.reject(new Error("Kara is not running"));
    }
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
    });
  }

  notify(method: string, params: unknown = {}): void {
    if (!this.closed) {
      this.write(JSON.stringify({ jsonrpc: "2.0", method, params }) + "\n");
    }
  }

  /** Feed raw stdout data (may contain partial or multiple lines). */
  receive(chunk: string): void {
    this.buffer += chunk;
    let nl: number;
    while ((nl = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, nl).trim();
      this.buffer = this.buffer.slice(nl + 1);
      if (line) {
        this.handleLine(line);
      }
    }
  }

  /** Reject everything outstanding (process exited). */
  close(reason = "Kara exited"): void {
    this.closed = true;
    for (const p of this.pending.values()) {
      p.reject(new Error(reason));
    }
    this.pending.clear();
  }

  private handleLine(line: string): void {
    let msg: any;
    try {
      msg = JSON.parse(line);
    } catch {
      this.emit("protocolError", `unparseable line from kara: ${line.slice(0, 200)}`);
      return;
    }
    if (msg === null || typeof msg !== "object" || msg.jsonrpc !== "2.0") {
      this.emit("protocolError", `invalid message: ${line.slice(0, 200)}`);
      return;
    }
    const hasMethod = typeof msg.method === "string";
    const hasId = msg.id !== undefined && msg.id !== null;
    if (hasMethod && hasId) {
      this.handleRequest(msg.id, msg.method, msg.params);
    } else if (hasMethod) {
      this.emit("notification", msg.method, msg.params);
    } else if (hasId) {
      const p = this.pending.get(msg.id);
      if (!p) {
        return;
      }
      this.pending.delete(msg.id);
      if (msg.error) {
        const e: RpcErrorObject = msg.error;
        p.reject(new RpcError(e.code, e.message, e.data));
      } else {
        p.resolve(msg.result);
      }
    }
  }

  private handleRequest(id: unknown, method: string, params: unknown): void {
    const respond = (result: unknown, error?: RpcErrorObject) => {
      const body = error ? { jsonrpc: "2.0", id, error } : { jsonrpc: "2.0", id, result };
      this.write(JSON.stringify(body) + "\n");
    };
    if (!this.requestHandler) {
      respond(undefined, { code: -32601, message: `no handler for ${method}` });
      return;
    }
    this.requestHandler(method, params).then(
      (r) => respond(r ?? null),
      (e) => respond(undefined, { code: -32603, message: String(e?.message ?? e) }),
    );
  }
}
