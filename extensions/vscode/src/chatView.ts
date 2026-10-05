// The chat sidebar. Renders agent events streamed from the local binary and
// sends user input back. Slash commands map to protocol methods.

import * as crypto from "crypto";
import * as vscode from "vscode";
import { AgentProcess } from "./agent";

export interface PromptOptions {
  mode?: "execute" | "plan" | "review";
  context?: unknown;
  approvePlan?: boolean;
  display?: string;
}

export class ChatViewProvider implements vscode.WebviewViewProvider {
  private view: vscode.WebviewView | undefined;
  private busy = false;
  private queue: unknown[] = [];

  constructor(
    private readonly extensionUri: vscode.Uri,
    private readonly getAgent: () => AgentProcess | undefined,
    private readonly actions: {
      viewDiff: (path: string) => Promise<void>;
      reject: (path: string) => Promise<void>;
      chooseModel: () => Promise<void>;
      doctor: () => Promise<void>;
      undo: () => Promise<void>;
      newSession: () => Promise<void>;
      showChanges: () => Promise<void>;
    },
  ) {}

  resolveWebviewView(view: vscode.WebviewView): void {
    this.view = view;
    view.webview.options = { enableScripts: true, localResourceRoots: [vscode.Uri.joinPath(this.extensionUri, "media")] };
    view.webview.html = this.html(view.webview);
    view.webview.onDidReceiveMessage((m) => this.onMessage(m));
    for (const m of this.queue) {
      void view.webview.postMessage(m);
    }
    this.queue = [];
  }

  post(message: unknown): void {
    if (this.view) {
      void this.view.webview.postMessage(message);
    } else {
      this.queue.push(message);
    }
  }

  reveal(): void {
    void vscode.commands.executeCommand("veyra.chat.focus");
  }

  updateStatus(): void {
    const agent = this.getAgent();
    this.post({
      type: "status",
      running: agent?.running ?? false,
      model: agent?.info?.model?.label ?? "",
      modelAvailable: agent?.info?.model?.available ?? false,
      profile: agent?.info?.profile ?? "",
      workspace: agent?.info?.workspace ?? "",
    });
  }

  async prompt(text: string, options: PromptOptions = {}): Promise<void> {
    const agent = this.getAgent();
    this.reveal();
    if (!agent || !agent.running) {
      this.post({ type: "error", text: "Veyra is not running. Open a folder and run \"Veyra: Restart Agent\"." });
      return;
    }
    if (this.busy) {
      this.post({ type: "error", text: "Veyra is still working. Stop the current task first." });
      return;
    }
    this.post({ type: "user", text: options.display ?? text, mode: options.mode ?? "execute" });
    this.setBusy(true);
    try {
      const result: any = await agent.request("session/prompt", {
        text,
        mode: options.mode ?? "execute",
        context: options.context,
        approvePlan: options.approvePlan ?? false,
      });
      this.post({ type: "result", result });
    } catch (e: any) {
      if (e?.code === -32002) {
        this.post({ type: "error", text: `${e.message}`, action: "chooseModel" });
      } else {
        this.post({ type: "error", text: String(e?.message ?? e) });
      }
    } finally {
      this.setBusy(false);
    }
  }

  private setBusy(b: boolean): void {
    this.busy = b;
    this.post({ type: "busy", busy: b });
  }

  private async onMessage(m: any): Promise<void> {
    const agent = this.getAgent();
    try {
      switch (m.type) {
        case "ready":
          this.updateStatus();
          break;
        case "send":
          await this.handleInput(String(m.text ?? ""));
          break;
        case "stop":
          agent?.client?.notify("session/cancel");
          await agent?.request("session/cancel").catch(() => undefined);
          break;
        case "viewDiff":
          await this.actions.viewDiff(m.path);
          break;
        case "reject":
          await this.actions.reject(m.path);
          break;
        case "approve":
          await this.prompt("", { approvePlan: true, display: "Approved the plan." });
          break;
        case "chooseModel":
          await this.actions.chooseModel();
          break;
        case "openFile":
          if (agent?.info?.workspace) {
            const uri = vscode.Uri.joinPath(vscode.Uri.file(agent.info.workspace), m.path);
            await vscode.window.showTextDocument(uri, { preview: true });
          }
          break;
      }
    } catch (e: any) {
      this.post({ type: "error", text: String(e?.message ?? e) });
    }
  }

  private async handleInput(input: string): Promise<void> {
    const text = input.trim();
    if (!text) {
      return;
    }
    if (!text.startsWith("/")) {
      await this.prompt(text);
      return;
    }
    const [cmd, ...rest] = text.split(/\s+/);
    const arg = rest.join(" ");
    const agent = this.getAgent();
    switch (cmd) {
      case "/plan":
      case "/morpheus":
        if (!arg) {
          this.post({ type: "info", text: `Usage: ${cmd} <task>. Veyra investigates and proposes a plan; nothing changes until you approve.` });
          return;
        }
        await this.prompt(arg, { mode: "plan", display: `${cmd} ${arg}` });
        return;
      case "/review":
      case "/oracle":
        await this.prompt(arg ? `Review my current diff, focusing on: ${arg}` : "Review my current diff.", { mode: "review", display: text });
        return;
      case "/approve":
        await this.prompt("", { approvePlan: true, display: "/approve" });
        return;
      case "/test":
        await this.prompt(arg ? `Run these tests and report the results: ${arg}` : "Run the project's tests and report the results.", { display: text });
        return;
      case "/lint":
        await this.prompt("Run the linter or type checker and report problems.", { display: text });
        return;
      case "/build":
        await this.prompt("Build the project and report the result.", { display: text });
        return;
      case "/undo":
        await this.actions.undo();
        return;
      case "/diff":
        await this.actions.showChanges();
        return;
      case "/model":
      case "/models":
        await this.actions.chooseModel();
        return;
      case "/doctor":
        await this.actions.doctor();
        return;
      case "/new":
        await this.actions.newSession();
        return;
      case "/stop":
        agent?.client?.notify("session/cancel");
        return;
      case "/clear":
      case "/compact":
      case "/permissions":
      case "/privacy":
      case "/matrix":
      case "/status": {
        if (!agent) {
          return;
        }
        const r: any =
          cmd === "/status"
            ? await agent.request("session/status")
            : await agent.request("session/command", { name: cmd.slice(1), args: arg });
        this.post({ type: "command", name: cmd, result: r });
        return;
      }
      case "/help":
        this.post({ type: "help", commands: agent?.info?.commands ?? [] });
        return;
      default:
        this.post({ type: "info", text: `Unknown command ${cmd}. Type /help.` });
    }
  }

  private html(webview: vscode.Webview): string {
    const nonce = crypto.randomBytes(16).toString("base64");
    const script = webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "media", "chat.js"));
    const style = webview.asWebviewUri(vscode.Uri.joinPath(this.extensionUri, "media", "chat.css"));
    return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link href="${style}" rel="stylesheet">
<title>Veyra</title>
</head>
<body>
<header id="status"><span id="model">starting…</span><span id="profile"></span></header>
<main id="log" aria-live="polite"></main>
<footer>
  <div id="working" hidden><span class="spinner"></span><span id="phase">working</span><button id="stop" title="Stop the current task">Stop</button></div>
  <textarea id="input" rows="3" placeholder="Ask Veyra to fix, explain, build or review. / for commands. Enter sends, Shift+Enter for a new line."></textarea>
</footer>
<script nonce="${nonce}" src="${script}"></script>
</body>
</html>`;
  }
}
