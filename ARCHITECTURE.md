# Architecture

There is exactly one implementation of Kara: **Kara Core**, a native Rust
binary (`kara`). The terminal UI is part of it; the VS Code extension is a
thin client that talks to it. There is no hosted backend.

```
CLI ─────────┐
             ├── Kara Core ── Agent
VS Code ─────┘   (kara)      ├─ Context
 (JSON-RPC                   ├─ Tools
  over stdio)                ├─ Git
                             ├─ Tests
                             ├─ Permissions
                             └─ InferenceProvider ──┬── local runtime (optional, this machine)
                                                    ├── another Kara machine (kara serve --inference)
                                                    └── OpenAI-compatible endpoint you control
```

The location of inference is irrelevant to the rest of Kara. The agent sees
one trait, `InferenceProvider`; everything specific to runtimes, model sizes,
GPU vendors, CUDA, Metal or llama.cpp lives behind it in `kara-inference`.
A test (`tests/integration/tests/architecture.rs`) fails if a Core crate
references them.

## Crates

| Crate | Responsibility |
|---|---|
| `kara-protocol` | `AgentEvent`, permission request types and JSON-RPC framing. The single vocabulary shared by the terminal UI and the editor. |
| `kara-core` | TOML configuration (platform directories, "repository may only tighten" rule, `kara config get/set`), permission modes, the privacy report, structured `TaskState`. |
| `kara-sandbox` | Resolves every path against the workspace (including symlinks and `..`), classifies shell commands into permission categories, recognizes secret paths, redacts credentials, flags prompt-injection text. |
| `kara-context` | Language packs (data in `languages/*.toml`), project command discovery, the SQLite repository index, ripgrep-style search, git helpers, task-to-file ranking. |
| `kara-tools` | 24 structured tools, each with a JSON schema, an *assessment* step (which permissions it needs, with a preview) and an execution step. Owns the undo journal. |
| `kara-agent` | The loop, prompts, approvers, session persistence, evaluation harness. Depends on the `InferenceProvider` trait only. |
| `kara-inference` | The `InferenceProvider` trait; `source::resolve` (which provider, or none with guidance); the OpenAI-compatible client; remote Kara (`serve --inference`, `connect`); and the optional `local` backend: hardware detection, model registry and recommendation, verified downloads, the llama.cpp runtime lifecycle. |
| `kara-cli` | The `kara` binary: terminal UI, slash commands, `serve --stdio` for editors, `serve --inference`, `connect`, `eval`, `doctor`, `config`. |

## The agent loop

One user request is a *turn*:

1. **Understand.** The agent refreshes the index and builds a short
   orientation: ranked files with reasons and symbol line numbers, git state
   (including the user's uncommitted files), detected test/lint/build
   commands and project conventions. File contents are not included; the
   model reads what it needs.
2. **Act.** The model calls tools. For each call Kara parses arguments
   leniently, asks the tool to *assess* the call, applies the permission
   policy, asks the user when required, runs the tool, records the result in
   the structured task state, and emits events.
3. **Verify.** Edits mark the task as unverified. If the model tries to finish
   with unverified edits and the project has a test command, Kara asks it to
   run tests first (once per turn).
4. **Recover.** A failing test run after edits counts as a recovery attempt.
   After `agent.max_recovery_attempts` the turn stops with a report instead of
   looping. This bounds one recovery cycle; it is not a usage limit.
5. **Finish.** The final answer, changed files, last test result and token
   usage are reported and persisted.

The model does not have to remember everything. `TaskState` (plan, hypothesis,
observations, files read and changed, test results, retries, completion
criteria) is rendered as a compact *working memory* block and attached to the
newest tool result on every model call. Older tool output is elided when the
conversation approaches the context window. Because the block is appended to
the end of the request, llama.cpp's prompt cache stays valid for the stable
prefix.

Modes: `execute` (default), `plan` (`/morpheus`, read-only tools, ends in
"awaiting approval"), and `review` (`/oracle`, read-only, the diff is
supplied up front).

## Context engine

* **Language packs** are data: extensions, manifests, test/lint/typecheck/
  format/build command discovery rules, symbol and import patterns, LSP and
  Tree-sitter hints. Unknown languages still get files, search, git and shell.
* **Index**: `cache/index-<hash>.db` in Kara's data directory (SQLite). A refresh walks the
  tree with `.gitignore` rules, compares size and mtime, and re-parses only
  changed files, in parallel. Huge and binary files are recorded by metadata
  only.
* **Symbols**: v0.1 extracts definitions with the pack's patterns behind a
  `SymbolExtractor` trait. A Tree-sitter backend is planned per language; it
  will replace the extractor without changing the index or the agent.
* **Ranking** for a task, strongest first: symbol matches, path matches,
  tests for the top files, importers of the top files, uncommitted changes,
  and a bounded lexical search when nothing else matched. No vector database:
  there is no benchmark yet showing one helps.

## Safety model

Permissions are enforced in Rust, outside the model. Repository content,
including `.kara/config.toml` in a cloned repository, cannot widen them. See
[SECURITY.md](SECURITY.md).

## Undo

Every mutation goes through the journal (`sessions/<id>/` in Kara's data directory). Before the
first write to a path in a turn, its exact bytes (or its absence) are stored
by content hash. `/undo` restores the latest turn's files, but only those
whose current content still matches what Kara wrote. Files the user edited
afterwards are reported and left alone. No git command is involved, so
pre-existing uncommitted work survives. Shell commands that modify files are
detected by comparing the worktree before and after; changes to files that
were clean in git are journaled using `git show HEAD:path` as the prior
content.

## Inference

`kara_inference::source::resolve` turns configuration into an
`InferenceSession`:

* **Endpoint providers** (`kara`, `openai_compat`, presets): probe the
  endpoint, pick the configured or first served model, and wrap it in the
  OpenAI-compatible client. Unreachable endpoints produce guidance, not a crash.
* **Local**: detect hardware, choose a model that fits (or none), ask before
  downloading, install the pinned runtime only after consent, start it on
  `127.0.0.1` with a free port, and stop it when Kara exits. Runtime processes
  orphaned by a killed Kara are reaped on the next start.

"No inference" is a normal outcome: the session carries guidance, and Kara
keeps working as an agent client. Hardware never gates Kara itself.

`kara serve --inference` exposes this machine's session over an
OpenAI-compatible HTTP API protected by a bearer token; `kara connect` points
another machine at it. See [INFERENCE.md](INFERENCE.md).

## Editor protocol

`kara serve --stdio` speaks newline-delimited JSON-RPC 2.0. The extension
sends `initialize`, `session/prompt`, `session/cancel` and similar requests;
Kara streams `event` notifications and asks for approvals with
`permission/request`. No network port is opened. See
[docs/PROTOCOL.md](docs/PROTOCOL.md).
