// Lifecycle of the `kara serve --stdio` child process. The extension holds
// no agent logic: everything goes through JSON-RPC to the local binary.

import * as cp from "child_process";
import * as vscode from "vscode";
import { RpcClient } from "./rpc";

export interface ModelInfo {
  available: boolean;
  label: string;
  id?: string | null;
  context?: number | null;
}

export interface InitializeResult {
  protocolVersion: string;
  karaVersion: string;
  workspace: string;
  session: string;
  profile: string;
  model: ModelInfo;
  commands: { name: string; description: string }[];
}

export class AgentProcess implements vscode.Disposable {
  private proc: cp.ChildProcess | undefined;
  client: RpcClient | undefined;
  info: InitializeResult | undefined;
  private readonly emitter = new vscode.EventEmitter<{ method: string; params: any }>();
  readonly onNotification = this.emitter.event;
  private readonly stateEmitter = new vscode.EventEmitter<void>();
  readonly onStateChange = this.stateEmitter.event;

  constructor(
    private readonly binary: string,
    private readonly output: vscode.OutputChannel,
    private readonly onPermission: (params: any) => Promise<{ decision: string }>,
  ) {}

  get running(): boolean {
    return this.proc !== undefined && this.proc.exitCode === null;
  }

  async start(workspace: string): Promise<InitializeResult> {
    this.stop();
    const proc = cp.spawn(this.binary, ["serve", "--stdio"], {
      cwd: workspace,
      stdio: ["pipe", "pipe", "pipe"],
      env: { ...process.env, NO_COLOR: "1" },
    });
    this.proc = proc;
    const client = new RpcClient((line) => proc.stdin?.write(line));
    this.client = client;
    proc.stdout?.setEncoding("utf8");
    proc.stdout?.on("data", (d: string) => client.receive(d));
    proc.stderr?.setEncoding("utf8");
    proc.stderr?.on("data", (d: string) => this.output.append(d));
    proc.on("exit", (code, signal) => {
      this.output.appendLine(`[kara exited: ${code ?? signal}]`);
      client.close("Kara exited");
      if (this.proc === proc) {
        this.proc = undefined;
        this.stateEmitter.fire();
      }
    });
    proc.on("error", (e) => this.output.appendLine(`[kara failed to start: ${e.message}]`));
    client.on("notification", (method: string, params: any) => {
      if (method === "log") {
        this.output.appendLine(`[${params?.level ?? "info"}] ${params?.message ?? ""}`);
      }
      this.emitter.fire({ method, params });
    });
    client.on("protocolError", (e: string) => this.output.appendLine(`[protocol] ${e}`));
    client.onRequest(async (method, params) => {
      if (method === "permission/request") {
        return this.onPermission(params);
      }
      throw new Error(`unsupported request ${method}`);
    });

    const profile = vscode.workspace.getConfiguration("kara").get<string>("permissionProfile", "");
    this.info = await client.request<InitializeResult>("initialize", {
      workspace,
      clientName: "vscode",
      ...(profile ? { profile } : {}),
    });
    this.stateEmitter.fire();
    return this.info;
  }

  request<T = any>(method: string, params: unknown = {}): Promise<T> {
    if (!this.client || !this.running) {
      return Promise.reject(new Error("Kara is not running. Run \"Kara: Restart Agent\"."));
    }
    return this.client.request<T>(method, params);
  }

  stop(): void {
    if (this.proc) {
      try {
        this.client?.notify("session/cancel");
        this.proc.stdin?.end();
        this.proc.kill();
      } catch {
        // already gone
      }
      this.proc = undefined;
      this.stateEmitter.fire();
    }
  }

  dispose(): void {
    this.stop();
    this.emitter.dispose();
    this.stateEmitter.dispose();
  }
}
