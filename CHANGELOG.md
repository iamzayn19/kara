# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Kara desktop app (Tauri) for macOS, Windows and Linux, a third client of
  `kara serve --stdio` alongside the terminal and VS Code: project picker and
  recent projects, sessions, model picker with download progress,
  Execute/Plan/Review modes with an Approve plan button, per-file diffs with
  reject, undo, context-usage pill with compaction, diagnostics and privacy
  report in Settings, and recovery when the Kara process exits.
- Desktop editor pane: gitignore-aware file explorer with changed-file
  badges, Monaco editor with syntax highlighting, side-by-side diffs, and
  editable files saved with Cmd/Ctrl+S. A save is refused if the file
  changed on disk since it was opened, so it cannot overwrite Kara's edits.
- Desktop integrated terminal (Cmd/Ctrl+J) on a real PTY, find in files
  (Shift+Cmd/Ctrl+F), quick open (Cmd/Ctrl+P) and a command palette
  (Shift+Cmd/Ctrl+P).
- Desktop installers: `.dmg` (Apple Silicon and Intel), `.exe`/`.msi` and
  `.deb`/`.AppImage`, each bundling its own `kara` CLI, built and
  install-tested by the `Desktop installers` workflow. Unsigned for now.
- Full-screen terminal UI for `kara` (ratatui).
- Editor protocol: `session/list`, `session/resume`, `fs/list`, `fs/read`,
  `fs/search`, `fs/write` and `fs/files` (see docs/PROTOCOL.md).
- "House of El" theme (red accent, blue for the user and live activity)
  across the desktop app, the terminal UI and the app icons; the VS Code
  extension now uses the Kara diamond mark.

### Changed

- Local inference checks current free memory before loading a model,
  refuses one that would need more than half the machine's RAM, and caps
  CPU threads by core count so weak machines stay responsive.
- The desktop app never falls back to the home directory as a project;
  it waits for a folder to be chosen.

### Security

- rustls 0.23.46, rustls-webpki 0.103.16 and webpki-roots 1.0.11
  (RUSTSEC-2026-0346/0347/0348/0349/0381/0382); tokio 1.54.1
  (RUSTSEC-2026-0386); zip 9 (RUSTSEC-2026-0367/0368/0369; Kara's own
  extraction was not exposed, regression tests added); ratatui 0.30, which
  drops the unmaintained `paste` (RUSTSEC-2024-0436).

## [0.1.0] - 2026-10-06

First public version.

### Added

- `kara` interactive terminal agent: natural-language tasks, streamed output,
  tool activity, inline diff previews, test summaries, permission prompts,
  slash-command completion, history and multi-line input.
- Engineering loop with structured task state (plan, hypothesis,
  observations, files, tests, retries), a verification gate after edits,
  bounded failure recovery, stall detection and context compaction.
- Modes: plan (`/morpheus`, `/plan`) with approval, and risk review
  (`/oracle`, `/review`) of the current diff.
- `/matrix`: repository statistics, active files and symbols, change
  ownership, model, context usage, tools, task and agent state.
- 24 structured tools: file reading, listing and search, symbol definitions and
  references, edits (search/replace and tolerant unified-diff patches),
  insertion by line, create, move and delete, shell, targeted test/lint/build runs, diagnostics
  and read-only git.
- Undo journal: `/undo` and `kara undo` restore Kara's last change batch and
  never overwrite pre-existing or later user changes. Covers file tools and
  files changed by shell commands where the prior content is recoverable.
- Permission modes `ask`, `workspace` (default) and `full`, with hard
  boundaries for secrets, privilege escalation, git push, destructive
  commands and paths outside the workspace in every mode. `full` is only ever
  enabled by the user; repository configuration can only make Kara stricter.
- Configuration in platform directories with `kara config get/set`, and
  per-session `--permissions`, `--provider` and `--model`.
- Command risk classification, path confinement with symlink checks, secret
  redaction in tool output, and prompt-injection labelling.
- Repository index (SQLite, incremental) and task-to-file ranking. 16 language
  packs as data: Ruby, Python, JavaScript, TypeScript, Rust, Go, Java, C, C++,
  C#, PHP, Swift, Kotlin, Shell, HTML/CSS, SQL.
- One inference interface with three kinds of provider: an optional local
  runtime, another Kara machine you own (`kara serve --inference` /
  `kara connect`, bearer-token protected), and OpenAI-compatible endpoints
  (Ollama, LM Studio, vLLM, others). Kara Core does not depend on any
  specific runtime, model or accelerator.
- Hardware-adaptive local inference: detection of RAM, CPU architecture,
  accelerators and disk; models chosen by measured or provisional quality,
  tool calling, memory, context, license and download size; when nothing
  fits, Kara keeps working and recommends compute you own elsewhere. Nothing
  is downloaded without consent.
- Local runtime: pinned, SHA-256-verified llama.cpp builds per platform,
  loopback-only lifecycle, cleanup of runtimes orphaned by a killed Kara.
- `kara privacy`, computed from the effective configuration.
- `kara eval`: offline evaluation over fixture repositories (Python, Ruby,
  TypeScript, Rust, Go, Java) with an oracle mode for CI.
- `kara serve --stdio`: JSON-RPC 2.0 protocol for editors.
- VS Code extension: a thin client of the Kara binary with chat, streamed
  events, tool activity, diff preview with accept/reject, permission prompts,
  inference selection and connection, doctor, selection and diagnostics
  context. Security-sensitive settings are user-only. The managed binary and
  models are shared with the CLI.
- Release archives for macOS (arm64, x86_64), Linux (x86_64, arm64) and
  Windows (x86_64) with `SHA256SUMS`; install scripts; generated Homebrew
  formula and winget manifests.
- Optional, offline training tooling under `training/` (not required to run
  Kara).
