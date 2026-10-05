# Architecture

Veyra is a single native binary (`veyra`) plus an optional VS Code extension
that talks to it. There is no hosted backend.

```
Terminal                                  VS Code
   │                                         │ extension (TypeScript, UI only)
   ▼                                         ▼ JSON-RPC 2.0 over stdio
┌───────────────────────── veyra binary (Rust) ─────────────────────────┐
│ veyra-cli      terminal UI, slash commands, `serve --stdio`, `eval`    │
│ veyra-agent    engineering loop, task state, verification, recovery    │
│ veyra-tools    structured tools, undo journal, process runner          │
│ veyra-context  language packs, incremental index, search, ranking      │
│ veyra-sandbox  path confinement, command classification, redaction     │
│ veyra-model    provider interface, OpenAI-compatible client, registry  │
│ veyra-runtime  llama.cpp install, verified downloads, server lifecycle │
│ veyra-core     config, permission policy, privacy report, task state   │
│ veyra-protocol events and JSON-RPC types shared by all front ends      │
└──────────────────────────────────┬─────────────────────────────────────┘
                                   │ HTTP on 127.0.0.1 (OpenAI-compatible)
                                   ▼
                     llama-server (child process) or Ollama /
                     LM Studio / vLLM / any compatible local server
```

## Crates

| Crate | Responsibility |
|---|---|
| `veyra-protocol` | `AgentEvent`, permission request types and JSON-RPC framing. The single vocabulary shared by the terminal UI and the editor. |
| `veyra-core` | TOML configuration with a "repository may only tighten" rule, permission profiles, the privacy report, structured `TaskState`. |
| `veyra-sandbox` | Resolves every path against the workspace (including symlinks and `..`), classifies shell commands into permission categories, recognizes secret paths, redacts credentials, flags prompt-injection text. |
| `veyra-context` | Language packs (data in `languages/*.toml`), project command discovery, the SQLite repository index, ripgrep-style search, git helpers, task-to-file ranking. |
| `veyra-tools` | 23 structured tools, each with a JSON schema, an *assessment* step (which permissions it needs, with a preview) and an execution step. Owns the undo journal. |
| `veyra-model` | `ModelProvider` trait; OpenAI-compatible streaming client; scripted provider for tests; model registry; hardware detection; model recommendation. |
| `veyra-runtime` | Locates or installs a pinned llama.cpp build, downloads models with resume and SHA-256 verification, launches `llama-server` bound to loopback and stops it. |
| `veyra-agent` | The loop, prompts, approvers, session persistence, evaluation harness. |
| `veyra-cli` | The `veyra` binary. |

## The agent loop

One user request is a *turn*:

1. **Understand.** The agent refreshes the index and builds a short
   orientation: ranked files with reasons and symbol line numbers, git state
   (including the user's uncommitted files), detected test/lint/build
   commands and project conventions. File contents are not included; the
   model reads what it needs.
2. **Act.** The model calls tools. For each call Veyra parses arguments
   leniently, asks the tool to *assess* the call, applies the permission
   policy, asks the user when required, runs the tool, records the result in
   the structured task state, and emits events.
3. **Verify.** Edits mark the task as unverified. If the model tries to finish
   with unverified edits and the project has a test command, Veyra asks it to
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
* **Index**: `~/.veyra/cache/index-<hash>.db` (SQLite). A refresh walks the
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
including `.veyra/config.toml` in a cloned repository, cannot widen them. See
[SECURITY.md](SECURITY.md).

## Undo

Every mutation goes through the journal (`~/.veyra/sessions/<id>/`). Before the
first write to a path in a turn, its exact bytes (or its absence) are stored
by content hash. `/undo` restores the latest turn's files, but only those
whose current content still matches what Veyra wrote. Files the user edited
afterwards are reported and left alone. No git command is involved, so
pre-existing uncommitted work survives. Shell commands that modify files are
detected by comparing the worktree before and after; changes to files that
were clean in git are journaled using `git show HEAD:path` as the prior
content.

## Model runtime

`veyra` locates `llama-server` (config path, PATH, or the managed install) or,
after asking, downloads the pinned build from `models/runtimes.toml` and
verifies its SHA-256. The server is started with `--host 127.0.0.1`, a free
port, `--jinja` and `--no-webui`, and is stopped when Veyra exits. Other
runtimes plug in through the same OpenAI-compatible provider. Nothing in the
agent depends on a specific model family.

## Editor protocol

`veyra serve --stdio` speaks newline-delimited JSON-RPC 2.0. The extension
sends `initialize`, `session/prompt`, `session/cancel` and similar requests;
Veyra streams `event` notifications and asks for approvals with
`permission/request`. No network port is opened. See
[docs/PROTOCOL.md](docs/PROTOCOL.md).
