// Builds the kara CLI in release mode and copies it into
// src-tauri/binaries/, which tauri.conf.json bundles into the app, so an
// installed .dmg/.exe works on a machine that has never installed the CLI.
import { execFileSync } from "node:child_process";
import { copyFileSync, chmodSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const desktop = join(dirname(fileURLToPath(import.meta.url)), "..");
const repo = join(desktop, "..", "..");
const exe = process.platform === "win32" ? "kara.exe" : "kara";

execFileSync("cargo", ["build", "--release", "--locked", "-p", "kara-cli"], { cwd: repo, stdio: "inherit" });

const dest = join(desktop, "src-tauri", "binaries");
mkdirSync(dest, { recursive: true });
copyFileSync(join(repo, "target", "release", exe), join(dest, exe));
if (process.platform !== "win32") chmodSync(join(dest, exe), 0o755);
console.log(`bundled ${exe} into src-tauri/binaries/`);
