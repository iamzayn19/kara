// Locate the kara binary, or download an official release from GitHub
// (after asking) and verify it against the release's SHA256SUMS.

import * as fs from "fs";
import * as https from "https";
import * as path from "path";
import * as cp from "child_process";
import * as vscode from "vscode";
import { REPO, archiveName, exeName, parseSha256Sums, releaseTarget, sha256File, which } from "./platform";

export async function locateBinary(context: vscode.ExtensionContext): Promise<string | undefined> {
  const configured = vscode.workspace.getConfiguration("kara").get<string>("binaryPath", "").trim();
  if (configured) {
    if (fs.existsSync(configured)) {
      return configured;
    }
    void vscode.window.showWarningMessage(`kara.binaryPath points to a missing file: ${configured}`);
  }
  const onPath = which(exeName());
  if (onPath) {
    return onPath;
  }
  const managed = path.join(context.globalStorageUri.fsPath, "bin", exeName());
  if (fs.existsSync(managed)) {
    return managed;
  }
  return undefined;
}

function get(url: string, redirects = 5): Promise<{ status: number; body: Buffer }> {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers: { "User-Agent": "kara-vscode", Accept: "*/*" } }, (res) => {
      if (res.statusCode && res.statusCode >= 300 && res.statusCode < 400 && res.headers.location && redirects > 0) {
        res.resume();
        resolve(get(new URL(res.headers.location, url).toString(), redirects - 1));
        return;
      }
      const chunks: Buffer[] = [];
      res.on("data", (c: Buffer) => chunks.push(c));
      res.on("end", () => resolve({ status: res.statusCode ?? 0, body: Buffer.concat(chunks) }));
      res.on("error", reject);
    });
    req.on("error", reject);
  });
}

/** Ask, then download the release matching this extension's version. */
export async function downloadBinary(context: vscode.ExtensionContext): Promise<string | undefined> {
  const target = releaseTarget();
  if (!target) {
    void vscode.window.showErrorMessage(`No prebuilt Kara for ${process.platform}/${process.arch}. Build it from source: https://github.com/${REPO}`);
    return undefined;
  }
  const version = String(context.extension.packageJSON.version);
  const asset = archiveName(version, target);
  const base = `https://github.com/${REPO}/releases/download/v${version}`;
  const choice = await vscode.window.showInformationMessage(
    `Kara's local agent binary was not found. Download the official release ${asset} from github.com/${REPO}? It is verified against the release's SHA256SUMS.`,
    { modal: true },
    "Download",
  );
  if (choice !== "Download") {
    return undefined;
  }
  return vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: "Downloading Kara", cancellable: false },
    async (progress) => {
      progress.report({ message: "checksums" });
      const sums = await get(`${base}/SHA256SUMS`);
      if (sums.status !== 200) {
        throw new Error(`could not fetch SHA256SUMS (HTTP ${sums.status})`);
      }
      const expected = parseSha256Sums(sums.body.toString("utf8")).get(asset);
      if (!expected) {
        throw new Error(`${asset} is not listed in SHA256SUMS`);
      }
      progress.report({ message: asset });
      const archive = await get(`${base}/${asset}`);
      if (archive.status !== 200) {
        throw new Error(`download failed (HTTP ${archive.status})`);
      }
      const dir = path.join(context.globalStorageUri.fsPath, "bin");
      fs.mkdirSync(dir, { recursive: true });
      const archivePath = path.join(dir, asset);
      fs.writeFileSync(archivePath, archive.body);
      const actual = await sha256File(archivePath);
      if (actual !== expected) {
        fs.rmSync(archivePath, { force: true });
        throw new Error(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}`);
      }
      progress.report({ message: "extracting" });
      // `tar` ships with macOS, Linux and Windows 10+ and handles both formats.
      cp.execFileSync("tar", ["-xf", archivePath, "-C", dir]);
      fs.rmSync(archivePath, { force: true });
      const binary = findFile(dir, exeName());
      if (!binary) {
        throw new Error(`${exeName()} not found in ${asset}`);
      }
      const dest = path.join(dir, exeName());
      if (binary !== dest) {
        fs.renameSync(binary, dest);
      }
      if (process.platform !== "win32") {
        fs.chmodSync(dest, 0o755);
      }
      return dest;
    },
  );
}

function findFile(dir: string, name: string): string | undefined {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      const found = findFile(p, name);
      if (found) {
        return found;
      }
    } else if (entry.name === name) {
      return p;
    }
  }
  return undefined;
}
