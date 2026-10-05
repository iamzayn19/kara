// Kara for VS Code: a UI over the local `kara` binary.

import * as path from "path";
import { execFile } from "child_process";
import * as vscode from "vscode";
import { AgentProcess } from "./agent";
import { downloadBinary, locateBinary } from "./binary";
import { ORIGINAL_SCHEME, OriginalContentProvider, pickAndShowChanges, rejectChange, showDiff } from "./changes";
import { ChatViewProvider } from "./chatView";
import { autoStart } from "./settings";

let agent: AgentProcess | undefined;

function workspaceFolder(): string | undefined {
  const active = vscode.window.activeTextEditor?.document.uri;
  if (active) {
    const f = vscode.workspace.getWorkspaceFolder(active);
    if (f) {
      return f.uri.fsPath;
    }
  }
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

/** Selection, file and diagnostics from the active editor. */
function editorContext(includeSelection: boolean): unknown {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    return undefined;
  }
  const root = workspaceFolder();
  const file = root ? path.relative(root, editor.document.uri.fsPath) : editor.document.uri.fsPath;
  const ctx: any = { file };
  const sel = editor.selection;
  if (includeSelection && !sel.isEmpty) {
    ctx.selection = {
      text: editor.document.getText(sel),
      startLine: sel.start.line + 1,
      endLine: sel.end.line + 1,
    };
  }
  if (vscode.workspace.getConfiguration("kara").get<boolean>("includeDiagnostics", true)) {
    const diags = vscode.languages.getDiagnostics(editor.document.uri).filter((d) => {
      return sel.isEmpty || !includeSelection || d.range.intersection(sel) !== undefined;
    });
    ctx.diagnostics = diags.slice(0, 30).map((d) => ({
      file,
      line: d.range.start.line + 1,
      severity: ["error", "warning", "info", "hint"][d.severity] ?? "error",
      message: d.message,
    }));
  }
  return ctx;
}

async function permission(params: any): Promise<{ decision: string }> {
  const kinds: string[] = (params.kinds ?? []).map((k: string) => k.replace(/_/g, "-"));
  const detail = [
    `Category: ${kinds.join(", ")}`,
    ...(params.reasons ?? []).map((r: string) => `• ${r}`),
    params.detail && params.detail.length < 1500 ? `\n${params.detail}` : "",
  ]
    .filter(Boolean)
    .join("\n");
  const buttons = params.can_remember ? ["Allow once", "Allow for session"] : ["Allow once"];
  const title = params.can_remember ? `Kara wants to: ${params.title}` : `Kara wants to (high risk): ${params.title}`;
  const choice = await vscode.window.showWarningMessage(title, { modal: true, detail }, ...buttons);
  if (choice === "Allow once") {
    return { decision: "allow_once" };
  }
  if (choice === "Allow for session") {
    return { decision: "allow_session" };
  }
  return { decision: "deny" };
}

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const output = vscode.window.createOutputChannel("Kara");
  const originals = new OriginalContentProvider();
  context.subscriptions.push(output, vscode.workspace.registerTextDocumentContentProvider(ORIGINAL_SCHEME, originals));

  const status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  status.command = "kara.open";
  context.subscriptions.push(status);
  const updateStatusBar = () => {
    if (!agent?.running) {
      status.text = "$(circle-slash) Kara";
      status.tooltip = "Kara agent is not running";
    } else if (!agent.info?.model?.available) {
      status.text = "$(warning) Kara: no model";
      status.tooltip = "Choose a local model";
    } else {
      status.text = `$(hubot) Kara`;
      status.tooltip = `Kara · ${agent.info.model.label} · permissions: ${agent.info.permissionsMode}`;
    }
    status.show();
  };

  const chat = new ChatViewProvider(context.extensionUri, () => agent, {
    viewDiff: async (p) => (agent ? showDiff(agent, originals, p) : undefined),
    reject: async (p) => (agent ? rejectChange(agent, p) : undefined),
    chooseModel: () => chooseModel(),
    connect: () => connectMachine(),
    ensureStarted: () => ensureStarted(),
    doctor: () => doctor(),
    undo: () => undo(),
    newSession: () => newSession(),
    showChanges: async () => (agent ? pickAndShowChanges(agent, originals) : undefined),
  });
  context.subscriptions.push(vscode.window.registerWebviewViewProvider("kara.chat", chat, { webviewOptions: { retainContextWhenHidden: true } }));

  let starting: Promise<void> | undefined;
  /** Start once; concurrent callers share the same start. */
  function ensureStarted(): Promise<void> {
    if (agent?.running) {
      return Promise.resolve();
    }
    starting ??= start().finally(() => (starting = undefined));
    return starting;
  }

  async function connectMachine(): Promise<void> {
    const url = await vscode.window.showInputBox({
      prompt: "Address of a machine running `kara serve --inference`",
      placeHolder: "http://192.168.1.20:7878",
      ignoreFocusOut: true,
    });
    if (!url) {
      return;
    }
    const token = await vscode.window.showInputBox({
      prompt: "Token printed by `kara serve --inference` on that machine",
      password: true,
      ignoreFocusOut: true,
    });
    if (!token) {
      return;
    }
    const binary = await locateBinary(context);
    if (!binary) {
      void vscode.window.showErrorMessage("The kara binary is not installed.");
      return;
    }
    // Kara Core does the work: verify the server, store the token, update config.
    const result = await new Promise<{ ok: boolean; out: string }>((resolve) => {
      execFile(binary, ["connect", url, "--token", token], (err, stdout, stderr) =>
        resolve({ ok: !err, out: `${stdout}${stderr}`.trim() }),
      );
    });
    output.appendLine(result.out);
    if (!result.ok) {
      void vscode.window.showErrorMessage(`Kara could not connect: ${result.out.split("\n").pop()}`);
      return;
    }
    void vscode.window.showInformationMessage(result.out.split("\n")[0]);
    await start();
  }

  async function start(): Promise<void> {
    const ws = workspaceFolder();
    if (!ws) {
      updateStatusBar();
      return;
    }
    let binary = await locateBinary(context);
    if (!binary) {
      try {
        binary = await downloadBinary(context);
      } catch (e: any) {
        void vscode.window.showErrorMessage(`Could not download Kara: ${e?.message ?? e}`);
      }
    }
    if (!binary) {
      chat.post({ type: "error", text: "The kara binary is not installed. Install it (see README) or run \"Kara: Restart Agent\" to download it." });
      updateStatusBar();
      return;
    }
    agent?.dispose();
    agent = new AgentProcess(binary, output, permission);
    agent.onNotification(({ method, params }) => {
      if (method === "event") {
        chat.post({ type: "event", event: params });
      }
    });
    agent.onStateChange(() => {
      updateStatusBar();
      chat.updateStatus();
    });
    try {
      const info = await agent.start(ws);
      output.appendLine(`Kara ${info.karaVersion} · ${info.workspace} · model: ${info.model.label}`);
      if (!info.model.available) {
        // Kara still works without inference; explain the options.
        chat.post({
          type: "guidance",
          text:
            info.model.guidance ??
            "No inference is configured yet. Choose a local model if this machine can run one, or connect to compute you own.",
        });
      }
    } catch (e: any) {
      output.appendLine(`initialize failed: ${e?.message ?? e}`);
      void vscode.window.showErrorMessage(`Kara failed to start: ${e?.message ?? e}`);
    }
    updateStatusBar();
    chat.updateStatus();
  }

  async function chooseModel(): Promise<void> {
    if (!agent?.running) {
      return;
    }
    const res: any = await agent.request("models/list");
    const items = res.models.map((m: any) => ({
      label: `${m.recommended ? "$(star-full) " : ""}${m.name} ${m.quantization}`,
      description: `${(m.sizeBytes / 1e9).toFixed(1)} GB · ${m.license}${m.installed ? " · installed" : ""}`,
      detail: m.reason ?? "",
      model: m,
    }));
    const pick: any = await vscode.window.showQuickPick(items, { placeHolder: res.summary, matchOnDetail: true });
    if (!pick) {
      return;
    }
    const m = pick.model;
    if (!m.installed) {
      const ok = await vscode.window.showInformationMessage(
        `Download ${m.name} ${m.quantization}?`,
        {
          modal: true,
          detail: `Size: ${(m.sizeBytes / 1e9).toFixed(1)} GB\nMemory needed: about ${((m.memoryNeeded ?? m.sizeBytes) / 1e9).toFixed(1)} GB\nLicense: ${m.license}\nSource: ${m.source} (revision ${String(m.revision).slice(0, 12)})\n\nThe download is verified against a pinned SHA-256. Progress appears in the Kara output channel.`,
        },
        "Download and use",
      );
      if (ok !== "Download and use") {
        return;
      }
    }
    output.show(true);
    await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: `Kara: starting ${m.name}`, cancellable: false },
      async () => {
        const model: any = await agent!.request("models/select", { id: m.id, download: true });
        if (agent?.info) {
          agent.info.model = model;
        }
      },
    );
    updateStatusBar();
    chat.updateStatus();
  }

  async function doctor(): Promise<void> {
    if (!agent?.running) {
      return;
    }
    const d: any = await agent.request("doctor");
    const hw = d.hardware;
    output.appendLine("── Kara doctor ──");
    output.appendLine(`OS: ${hw.os_version} (${hw.arch}) · CPU: ${hw.cpu} (${hw.cpu_cores} threads)`);
    output.appendLine(`RAM: ${(hw.total_ram / 2 ** 30).toFixed(1)} GB total, ${(hw.available_ram / 2 ** 30).toFixed(1)} GB available`);
    for (const g of hw.gpus) {
      output.appendLine(`GPU: ${g.name}${g.vram_bytes ? `, ${(g.vram_bytes / 2 ** 30).toFixed(1)} GB VRAM` : ""}`);
    }
    output.appendLine(`Metal: ${hw.metal} · CUDA: ${hw.cuda} · Vulkan: ${hw.vulkan} · ROCm: ${hw.rocm} · unified memory: ${hw.unified_memory}`);
    output.appendLine(`llama.cpp: ${d.llamaCpp ?? "not installed"}`);
    output.appendLine(`Recommendation: ${d.recommendation.summary}`);
    output.show(true);
  }

  async function undo(): Promise<void> {
    if (!agent?.running) {
      return;
    }
    try {
      const r: any = await agent.request("session/undo");
      const parts = [];
      if (r.restored.length) parts.push(`restored ${r.restored.join(", ")}`);
      if (r.conflicts.length) parts.push(`kept (edited after Kara): ${r.conflicts.join(", ")}`);
      chat.post({ type: "info", text: `Undo: ${parts.join("; ") || "nothing to restore"}` });
    } catch (e: any) {
      chat.post({ type: "info", text: String(e?.message ?? e) });
    }
  }

  async function newSession(): Promise<void> {
    if (!agent?.running) {
      return;
    }
    await agent.request("session/new");
    chat.post({ type: "info", text: "Started a new session." });
  }

  const reg = (id: string, fn: (...a: any[]) => unknown) => context.subscriptions.push(vscode.commands.registerCommand(id, fn));
  reg("kara.open", async () => {
    chat.reveal();
    await ensureStarted();
  });
  reg("kara.connect", () => connectMachine());
  reg("kara.newSession", () => newSession());
  reg("kara.askAboutSelection", async () => {
    const q = await vscode.window.showInputBox({ prompt: "Ask Kara about the selected code", placeHolder: "What does this do? Why could it fail?" });
    if (q) {
      await chat.prompt(q, { context: editorContext(true), display: `${q} (about the selection)` });
    }
  });
  reg("kara.fixSelection", async () => {
    await chat.prompt("Fix the problems in the selected code. Explain the cause, make the fix, and run the relevant tests.", {
      context: editorContext(true),
      display: "Fix the selected code",
    });
  });
  reg("kara.reviewDiff", () => chat.prompt("Review my current diff.", { mode: "review", display: "/oracle" }));
  reg("kara.runTests", () => chat.prompt("Run the project's tests and report the results.", { display: "/test" }));
  reg("kara.chooseModel", () => chooseModel());
  reg("kara.doctor", () => doctor());
  reg("kara.showChanges", () => (agent ? pickAndShowChanges(agent, originals) : undefined));
  reg("kara.undo", () => undo());
  reg("kara.stop", () => agent?.client?.notify("session/cancel"));
  reg("kara.restart", () => start());

  context.subscriptions.push({ dispose: () => agent?.dispose() });
  updateStatusBar();
  if (autoStart()) {
    void start();
  }
}

export function deactivate(): void {
  agent?.dispose();
}
