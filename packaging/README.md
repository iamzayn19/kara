# Packaging

Kara ships as one native binary per platform. Package managers install that
binary; none of them adds a Node.js or Python runtime dependency.

| Channel | How | Status |
|---|---|---|
| GitHub Releases | `kara-v<version>-<target>.tar.gz` / `.zip` + `SHA256SUMS` | built by `.github/workflows/release.yml` |
| Install scripts | `scripts/install.sh` (macOS, Linux), `scripts/install.ps1` (Windows); both verify `SHA256SUMS` | ready |
| Homebrew | formula rendered per release (`homebrew/kara.rb`) for the tap `iamzayn19/homebrew-kara` | formula generated; tap to be created at first release |
| winget | manifests rendered per release (`winget/manifests/i/iamzayn19/Kara/<version>/`) for a pull request to `microsoft/winget-pkgs` | manifests generated; submission at first release |
| VS Code | `kara-<version>.vsix`; the extension finds `kara` on PATH or downloads the release binary into Kara's data directory | VSIX built per release; Marketplace publishing later |

`render.py` produces the Homebrew formula and winget manifests from a
release's `SHA256SUMS`:

```sh
python3 packaging/render.py --version 0.1.0 --sums dist/SHA256SUMS --out dist/packaging
```

An npm package, if ever added, would only be an optional bootstrapper that
downloads the native binary. There is no Python package: Kara has no Python
SDK.
