"""Render package-manager manifests for a Kara release from its SHA256SUMS.

    python3 packaging/render.py --version 0.1.0 --sums dist/SHA256SUMS --out dist/packaging

Writes:
  homebrew/kara.rb                                   formula for a tap (iamzayn19/homebrew-kara)
  winget/manifests/i/iamzayn19/Kara/<version>/*.yaml manifests for winget-pkgs

Both install the native kara binary; neither needs Node.js or Python at run time.
"""

import argparse
import os
import sys

REPO = "iamzayn19/kara"
DESCRIPTION = "Local-first coding agent. CLI and VS Code. Your code, your compute."

TARGETS = {
    "macos_arm": "aarch64-apple-darwin",
    "macos_intel": "x86_64-apple-darwin",
    "linux_arm": "aarch64-unknown-linux-gnu",
    "linux_intel": "x86_64-unknown-linux-gnu",
    "windows_x64": "x86_64-pc-windows-msvc",
}


def asset(version, target):
    ext = "zip" if "windows" in target else "tar.gz"
    return f"kara-v{version}-{target}.{ext}"


def url(version, target):
    return f"https://github.com/{REPO}/releases/download/v{version}/{asset(version, target)}"


def parse_sums(text):
    sums = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) == 2 and len(parts[0]) == 64:
            sums[parts[1].lstrip("*")] = parts[0].lower()
    return sums


def homebrew(version, sums):
    def block(key):
        t = TARGETS[key]
        return f'      url "{url(version, t)}"\n      sha256 "{sums[asset(version, t)]}"'

    linux_arm = ""
    if asset(version, TARGETS["linux_arm"]) in sums:
        linux_arm = f"""    on_arm do
{block("linux_arm")}
    end
"""
    return f'''class Kara < Formula
  desc "{DESCRIPTION}"
  homepage "https://github.com/{REPO}"
  version "{version}"
  license "Apache-2.0"

  on_macos do
    on_arm do
{block("macos_arm")}
    end
    on_intel do
{block("macos_intel")}
    end
  end

  on_linux do
{linux_arm}    on_intel do
{block("linux_intel")}
    end
  end

  def install
    bin.install "kara"
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/kara --version")
    system bin/"kara", "privacy"
  end
end
'''


def winget(version, sums):
    ident = "iamzayn19.Kara"
    t = TARGETS["windows_x64"]
    folder = asset(version, t)[: -len(".zip")]
    header = "# yaml-language-server: $schema=https://aka.ms/winget-manifest.{kind}.1.6.0.schema.json\n"
    version_manifest = header.format(kind="version") + f"""PackageIdentifier: {ident}
PackageVersion: {version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
"""
    installer = header.format(kind="installer") + f"""PackageIdentifier: {ident}
PackageVersion: {version}
InstallerType: zip
NestedInstallerType: portable
NestedInstallerFiles:
  - RelativeFilePath: {folder}\\kara.exe
    PortableCommandAlias: kara
Installers:
  - Architecture: x64
    InstallerUrl: {url(version, t)}
    InstallerSha256: {sums[asset(version, t)].upper()}
ManifestType: installer
ManifestVersion: 1.6.0
"""
    locale = header.format(kind="defaultLocale") + f"""PackageIdentifier: {ident}
PackageVersion: {version}
PackageLocale: en-US
Publisher: iamzayn19
PublisherUrl: https://github.com/iamzayn19
PackageName: Kara
PackageUrl: https://github.com/{REPO}
License: Apache-2.0
LicenseUrl: https://github.com/{REPO}/blob/main/LICENSE
ShortDescription: {DESCRIPTION}
Moniker: kara
Tags:
  - coding-agent
  - cli
  - developer-tools
ReleaseNotesUrl: https://github.com/{REPO}/releases/tag/v{version}
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"""
    return {
        f"{ident}.yaml": version_manifest,
        f"{ident}.installer.yaml": installer,
        f"{ident}.locale.en-US.yaml": locale,
    }


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--version", required=True)
    ap.add_argument("--sums", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    sums = parse_sums(open(args.sums, encoding="utf-8").read())
    required = ["macos_arm", "macos_intel", "linux_intel", "windows_x64"]
    missing = [asset(args.version, TARGETS[k]) for k in required if asset(args.version, TARGETS[k]) not in sums]
    if missing:
        sys.exit(f"missing from SHA256SUMS: {', '.join(missing)}")

    hb = os.path.join(args.out, "homebrew")
    os.makedirs(hb, exist_ok=True)
    with open(os.path.join(hb, "kara.rb"), "w", encoding="utf-8") as f:
        f.write(homebrew(args.version, sums))

    wg = os.path.join(args.out, "winget", "manifests", "i", "iamzayn19", "Kara", args.version)
    os.makedirs(wg, exist_ok=True)
    for name, text in winget(args.version, sums).items():
        with open(os.path.join(wg, name), "w", encoding="utf-8") as f:
            f.write(text)
    print(f"wrote {hb}/kara.rb and {wg}/")
    return 0


if __name__ == "__main__":
    sys.exit(main())
