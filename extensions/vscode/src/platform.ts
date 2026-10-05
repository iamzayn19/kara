// Platform helpers for locating and verifying the kara binary.
// No VS Code imports: unit-tested with node:test.

import * as crypto from "crypto";
import * as fs from "fs";
import * as path from "path";

export const REPO = "iamzayn19/kara";

/** Rust target triple used in Kara release asset names. */
export function releaseTarget(platform: NodeJS.Platform = process.platform, arch: string = process.arch): string | undefined {
  const key = `${platform}-${arch}`;
  const map: Record<string, string> = {
    "darwin-arm64": "aarch64-apple-darwin",
    "darwin-x64": "x86_64-apple-darwin",
    "linux-x64": "x86_64-unknown-linux-gnu",
    "linux-arm64": "aarch64-unknown-linux-gnu",
    "win32-x64": "x86_64-pc-windows-msvc",
  };
  return map[key];
}

export function exeName(platform: NodeJS.Platform = process.platform): string {
  return platform === "win32" ? "kara.exe" : "kara";
}

export function archiveName(version: string, target: string): string {
  const ext = target.includes("windows") ? "zip" : "tar.gz";
  return `kara-v${version}-${target}.${ext}`;
}

/** Find an executable on PATH. */
export function which(name: string, envPath = process.env.PATH ?? ""): string | undefined {
  for (const dir of envPath.split(path.delimiter)) {
    if (!dir) {
      continue;
    }
    const candidate = path.join(dir, name);
    try {
      const st = fs.statSync(candidate);
      if (st.isFile()) {
        return candidate;
      }
    } catch {
      // not here
    }
  }
  return undefined;
}

/** Parse a SHA256SUMS file ("<hex>  <name>" per line). */
export function parseSha256Sums(text: string): Map<string, string> {
  const out = new Map<string, string>();
  for (const line of text.split(/\r?\n/)) {
    const m = line.trim().match(/^([0-9a-fA-F]{64})\s+\*?(.+)$/);
    if (m) {
      out.set(m[2].trim(), m[1].toLowerCase());
    }
  }
  return out;
}

export function sha256File(file: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const h = crypto.createHash("sha256");
    fs.createReadStream(file)
      .on("data", (d) => h.update(d))
      .on("error", reject)
      .on("end", () => resolve(h.digest("hex")));
  });
}
