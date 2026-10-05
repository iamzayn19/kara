# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - unreleased

First public version. Not yet released: see docs/RELEASE_CHECKLIST.md for the
remaining release blockers.

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
- Permission profiles (safe, balanced, autonomous) with hard boundaries for
  secrets, privilege escalation, git push, destructive commands and paths
  outside the workspace. Repository configuration can only make Kara stricter.
- Command risk classification, path confinement with symlink checks, secret
  redaction in tool output, and prompt-injection labelling.
- Repository index (SQLite, incremental) and task-to-file ranking. 16 language
  packs as data: Ruby, Python, JavaScript, TypeScript, Rust, Go, Java, C, C++,
  C#, PHP, Swift, Kotlin, Shell, HTML/CSS, SQL.
- Managed llama.cpp runtime: pinned, SHA-256-verified prebuilt builds per
  platform, loopback-only server lifecycle. OpenAI-compatible providers for
  Ollama, LM Studio, vLLM and others.
- Model registry as data with pinned revisions and checksums; hardware
  detection; `kara doctor`; `/model auto` selection by measured or
  provisional quality, tool calling, memory, context, license and download
  size; consent before every download.
- `kara privacy`, computed from the effective configuration.
- `kara eval`: offline evaluation over fixture repositories (Python, Ruby,
  TypeScript, Rust, Go, Java) with an oracle mode for CI.
- `kara serve --stdio`: JSON-RPC 2.0 protocol for editors.
- VS Code extension: chat sidebar, streamed events, tool activity, diff
  previews with per-file reject, permission dialogs, model selection, doctor,
  selection and diagnostics context, and commands for the common actions.
- Optional, offline training tooling under `training/` (not required to run
  Kara).
