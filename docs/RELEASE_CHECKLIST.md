# Release checklist: v0.1.0

`v0.1.0` was tagged on 2026-10-06 after every blocker below was checked. Status as of
2026-10-06. "Local" means verified on the maintainer's machine (macOS 26,
Apple M5, 16 GB); "CI" means it is enforced by `.github/workflows/ci.yml` and
needs a green run on GitHub.

| Blocker | Status | Evidence |
|---|---|---|
| Linux build passes | ✅ CI | `test (ubuntu-latest)` job; release workflow builds x86_64 and arm64 |
| macOS build passes | ✅ local · ✅ CI | `cargo build --release` (arm64) |
| Windows build passes | ✅ CI | `test (windows-latest)` job |
| Unit tests pass | ✅ local · ✅ CI | `cargo test --workspace` |
| Integration tests pass | ✅ local · ✅ CI | `tests/integration` (agent loop, undo, security, fixtures, scale) |
| Agent fixture tests pass | ✅ local | fixture validation + `kara eval --oracle` (Go only in CI) |
| VS Code extension builds | ✅ local · ✅ CI | `npm test`, `npm run package` |
| VSIX installs successfully | ✅ local · ✅ CI | `code --install-extension kara-0.1.0.vsix` into an isolated extensions dir |
| Kara CLI works outside repository checkout | ✅ local | release binary copied elsewhere with a fresh `KARA_HOME`; links only system libraries |
| Hardware detection works | ✅ local | `kara doctor` (Metal, unified memory, RAM, disk) |
| Model download works | ✅ local | `kara models pull qwen3-4b-q4_k_m` (2.5 GB, Hugging Face, pinned revision) |
| Checksum verification works | ✅ local | download verified against pinned SHA-256; mismatch/resume covered by tests |
| llama.cpp lifecycle works | ✅ local | pinned b11396 installed, started on 127.0.0.1, health-checked, stopped |
| Model inference works | ✅ local | Qwen3-4B via llama.cpp (terminal and JSON-RPC) |
| Tool calling works | ✅ local | native tool calls (read, edit, run_test) from Qwen3-4B |
| Repository edits work | ✅ local | real fix in the python-shop fixture |
| Tests can be executed | ✅ local | targeted `run_test` with parsed results |
| Failure recovery works | ✅ local | scripted tests; real model recovered in eval reruns (docs/MODEL_EVALUATION.md) |
| Dogfood session on Kara itself | ✅ local | docs/DOGFOOD.md (explain, add a test, run tests, /diff, /oracle, /undo, clean state) |
| /undo preserves user changes | ✅ local | `tests/integration/tests/undo.rs` |
| /matrix works | ✅ local | interactive sessions with and without a model (docs/DOGFOOD.md) |
| /morpheus works | ✅ local (scripted) | `plan_mode_is_read_only_and_waits_for_approval`; real-model plan session still to record |
| /oracle works | ✅ local | scripted test plus real-model review in the dogfood session |
| Permission boundaries tested | ✅ local | `tests/integration/tests/security.rs`, sandbox unit tests |
| Kara runs without local inference | ✅ local | fresh `KARA_HOME`: guidance instead of failure, nothing downloaded; `weak_machine_*` tests |
| Remote Kara inference | ✅ local | `kara serve --inference` + `kara connect` between two Kara homes; wrong token rejected; owner-only token files; real task completed |
| Core independent of runtimes/accelerators | ✅ | `tests/integration/tests/architecture.rs` |
| Permission modes ask/workspace/full; full never from repository or workspace config | ✅ local · ✅ CI (VS Code host test) | `kara-core` tests, `security.rs`, protocol test, extension host test |
| Installers verify checksums | ✅ local (install.sh, file:// mirror) · ✅ install.ps1 under PowerShell 7.4.6 on macOS (happy path installs; tampered SHA256SUMS rejected) · ❌ not run on real Windows | `scripts/install.sh` with `KARA_RELEASE_BASE` |
| Homebrew formula / winget manifests | ✅ published: tap `iamzayn19/homebrew-kara`; winget PR microsoft/winget-pkgs#447524 (pending Microsoft review) | `packaging/render.py` |
| Privacy command accurate | ✅ local | derived from effective config; remote endpoints reported |
| No telemetry exists by default | ✅ | no telemetry code; network uses listed in PRIVACY.md |
| No proprietary API required | ✅ | local llama.cpp; OpenAI-compatible protocol only |
| No secret committed | ✅ CI (gitleaks) | gitleaks job; manual review |
| Dependency licenses audited | ✅ local · ✅ CI | docs/LICENSES.md, cargo-deny job |
| README commands actually executed | ✅ local | see "README command log" below |
| No assistant/model attribution appears as author | ✅ | metadata: iamzayn19 only |
| Full clean-machine installation tested | ✅ | Linux aarch64: fresh `ubuntu:24.04` container. macOS: install and first run on this Mac with Kara data moved aside. Windows 11 ARM64: GitHub `windows-11-arm` runner runs the real `scripts/install.ps1` (run 37459873003): checksum verified, `kara --version`, `privacy`, `doctor` and the no-model first run all pass. All install paths were checked from a local release-shaped mirror; the public GitHub Release download is re-checked after publishing |
| GitHub CI green on Linux, macOS and Windows | ✅ | all jobs green on 06fb176 (run 37438226296) |

## README command log

Run with the release binary on 2026-10-05:
`kara --version`, `kara doctor`, `kara models`, `kara privacy`,
`kara config path`, `kara models pull qwen3-4b-q4_k_m`, `kara run ...`,
interactive `kara` with `/help`, `/matrix`, `/permissions`, `/git`, `/test`,
`/diff`, `/oracle`, `/undo`, `/exit`, and `kara eval --oracle`. The install
scripts and `cargo install --git` need the public repository and a release;
test them during the clean-machine check.

## Publishing steps (after all blockers pass)

1. Update CHANGELOG.md (release date) and docs/MODEL_EVALUATION.md.
2. `git tag v0.1.0 && git push origin v0.1.0`. The release workflow builds the
   binaries for all targets, `SHA256SUMS` and the VSIX, and creates a **draft**
   GitHub Release.
3. Download the draft's assets on a clean machine; run the install script,
   `kara doctor` and one real task; install the VSIX.
4. Publish the draft release.
5. Extension marketplaces: add the `VSCE_PAT` and/or `OVSX_PAT` repository
   secrets (created by the publisher account, never committed), then re-run
   the `publish-extension` job, or publish by hand:
   `npx @vscode/vsce publish --packagePath kara-0.1.0.vsix` and
   `npx ovsx publish kara-0.1.0.vsix -p <token>`.
