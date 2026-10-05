// Runs inside the VS Code extension host (via @vscode/test-electron).
import * as assert from "assert";
import * as vscode from "vscode";

export async function run(): Promise<void> {
  const ext = vscode.extensions.getExtension("iamzayn19.kara");
  assert.ok(ext, "extension is installed");
  await ext!.activate();
  assert.ok(ext!.isActive, "extension activates");
  const commands = await vscode.commands.getCommands(true);
  for (const id of [
    "kara.open",
    "kara.newSession",
    "kara.askAboutSelection",
    "kara.fixSelection",
    "kara.reviewDiff",
    "kara.runTests",
    "kara.chooseModel",
    "kara.doctor",
    "kara.connect",
  ]) {
    assert.ok(commands.includes(id), `command ${id} is registered`);
  }
  // A workspace must not be able to escalate permissions.
  const config = vscode.workspace.getConfiguration("kara");
  let workspaceWriteRejected = false;
  try {
    await config.update("permissions.mode", "full", vscode.ConfigurationTarget.Workspace);
  } catch {
    workspaceWriteRejected = true;
  }
  const { userSetting } = await import("../../settings");
  assert.ok(
    workspaceWriteRejected || userSetting<string>("permissions.mode", "") !== "full",
    "workspace settings cannot set kara.permissions.mode",
  );
  await vscode.commands.executeCommand("kara.open");
}
