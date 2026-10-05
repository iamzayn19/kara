// Settings access. Security-sensitive settings are read from the user (and
// default) scope only, so a repository's .vscode/settings.json can never
// escalate permissions, redirect inference or swap the Kara binary. The
// manifest also declares them application/machine-scoped; this is the second
// line of defence.

import * as vscode from "vscode";

/** Value from user settings, ignoring workspace and folder values. */
export function userSetting<T>(key: string, fallback: T): T {
  const info = vscode.workspace.getConfiguration("kara").inspect<T>(key);
  if (info?.globalValue !== undefined) {
    return info.globalValue;
  }
  return info?.defaultValue ?? fallback;
}

export interface SessionSettings {
  permissionsMode: string;
  inferenceProvider: string;
  model: string;
}

export function sessionSettings(): SessionSettings {
  return {
    permissionsMode: userSetting<string>("permissions.mode", ""),
    inferenceProvider: userSetting<string>("inference.provider", ""),
    model: userSetting<string>("model", ""),
  };
}

export function binaryPathSetting(): string {
  return userSetting<string>("binary.path", "").trim();
}

export function autoStart(): boolean {
  return vscode.workspace.getConfiguration("kara").get<boolean>("autoStart", true);
}
