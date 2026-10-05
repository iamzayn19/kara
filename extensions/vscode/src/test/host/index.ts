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
  ]) {
    assert.ok(commands.includes(id), `command ${id} is registered`);
  }
  await vscode.commands.executeCommand("kara.open");
}
