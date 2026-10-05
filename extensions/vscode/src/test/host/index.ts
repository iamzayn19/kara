// Runs inside the VS Code extension host (via @vscode/test-electron).
import * as assert from "assert";
import * as vscode from "vscode";

export async function run(): Promise<void> {
  const ext = vscode.extensions.getExtension("iamzayn19.veyra");
  assert.ok(ext, "extension is installed");
  await ext!.activate();
  assert.ok(ext!.isActive, "extension activates");
  const commands = await vscode.commands.getCommands(true);
  for (const id of [
    "veyra.open",
    "veyra.newSession",
    "veyra.askAboutSelection",
    "veyra.fixSelection",
    "veyra.reviewDiff",
    "veyra.runTests",
    "veyra.chooseModel",
    "veyra.doctor",
  ]) {
    assert.ok(commands.includes(id), `command ${id} is registered`);
  }
  await vscode.commands.executeCommand("veyra.open");
}
