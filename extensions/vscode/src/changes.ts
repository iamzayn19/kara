// Diff previews for Veyra's changes, and accept/reject per file.

import * as path from "path";
import * as vscode from "vscode";
import { AgentProcess } from "./agent";

export const ORIGINAL_SCHEME = "veyra-original";

interface Change {
  path: string;
  before: string | null;
  after: string | null;
  diff: string;
}

/** Serves the pre-Veyra content of files for the diff editor. */
export class OriginalContentProvider implements vscode.TextDocumentContentProvider {
  private originals = new Map<string, string>();
  private readonly emitter = new vscode.EventEmitter<vscode.Uri>();
  readonly onDidChange = this.emitter.event;

  set(rel: string, content: string): vscode.Uri {
    this.originals.set(rel, content);
    const uri = vscode.Uri.from({ scheme: ORIGINAL_SCHEME, path: "/" + rel });
    this.emitter.fire(uri);
    return uri;
  }

  provideTextDocumentContent(uri: vscode.Uri): string {
    return this.originals.get(uri.path.replace(/^\//, "")) ?? "";
  }
}

export async function fetchChanges(agent: AgentProcess): Promise<{ veyra: Change[]; user: string[] }> {
  return agent.request("session/changes");
}

export async function showDiff(agent: AgentProcess, provider: OriginalContentProvider, rel: string): Promise<void> {
  const changes = await fetchChanges(agent);
  const change = changes.veyra.find((c) => c.path === rel);
  if (!change) {
    void vscode.window.showInformationMessage(`${rel} has no pending Veyra changes.`);
    return;
  }
  const root = agent.info?.workspace ?? "";
  const left = provider.set(rel, change.before ?? "");
  const right = vscode.Uri.file(path.join(root, rel));
  await vscode.commands.executeCommand("vscode.diff", left, right, `${rel} (before Veyra ↔ now)`);
}

export async function rejectChange(agent: AgentProcess, rel: string): Promise<void> {
  const ok = await vscode.window.showWarningMessage(
    `Revert Veyra's changes to ${rel}? Your own edits are never overwritten: if you changed the file after Veyra, the revert is refused.`,
    { modal: true },
    "Revert",
  );
  if (ok !== "Revert") {
    return;
  }
  await agent.request("session/command", { name: "revert", args: rel });
  void vscode.window.showInformationMessage(`Reverted ${rel}.`);
}

export async function pickAndShowChanges(agent: AgentProcess, provider: OriginalContentProvider): Promise<void> {
  const changes = await fetchChanges(agent);
  if (changes.veyra.length === 0) {
    void vscode.window.showInformationMessage("Veyra has not changed any files in this session.");
    return;
  }
  const pick = await vscode.window.showQuickPick(
    changes.veyra.map((c) => ({ label: c.path, description: diffStat(c.diff) })),
    { placeHolder: "Veyra's changes (your own uncommitted changes are not listed)" },
  );
  if (pick) {
    await showDiff(agent, provider, pick.label);
  }
}

export function diffStat(diff: string): string {
  let add = 0;
  let del = 0;
  for (const l of diff.split("\n")) {
    if (l.startsWith("+") && !l.startsWith("+++")) {
      add++;
    } else if (l.startsWith("-") && !l.startsWith("---")) {
      del++;
    }
  }
  return `+${add} −${del}`;
}
