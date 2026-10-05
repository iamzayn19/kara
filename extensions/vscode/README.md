# Kara for VS Code

Kara runs on your machine. Intelligence runs wherever your compute is.

This extension is the VS Code interface to Kara, a local-first coding agent.
It is a thin client: all agent, context, git, file-editing, permission and
inference logic lives in the `kara` binary, which the extension starts and
talks to over JSON-RPC on stdio.

No account. No subscription. No usage quotas. No telemetry.

## Features

* **Chat** in the Kara activity bar, with streamed responses and live tool
  activity, test results and file changes.
* **Diff preview, accept and reject** for every file Kara changes. Your own
  edits are never overwritten.
* **Permission prompts** for edits, shell commands, network access and other
  risky actions, depending on the permission mode.
* **Editor context**: *Kara: Ask About Selection* and *Kara: Fix Selection*
  send the selected code and the file's diagnostics.
* **Commands in chat**: `/matrix`, `/morpheus <task>` (plan, then approve),
  `/oracle` (risk review of your diff), `/model`, `/models`, `/status`,
  `/undo`, `/diff`, `/connect` and more.
* **Inference wherever you have it**: a local model if this machine can run
  one, another machine you own (*Kara: Connect to Inference Machine*), or an
  OpenAI-compatible endpoint. The extension never downloads a model without
  your confirmation.

## The Kara binary

On start the extension uses, in order: `kara.binary.path`, `kara` on your
PATH, or the copy in Kara's data directory. If none exists, it offers to
download the release for your platform from GitHub and verifies it against
the release's `SHA256SUMS`. The CLI and the extension share the same binary
location and the same downloaded models.

## Settings

| Setting | |
|---|---|
| `kara.permissions.mode` | `ask`, `workspace` or `full` for editor sessions; empty uses your Kara configuration. User setting only. |
| `kara.inference.provider` | `local`, `kara`, `openai_compat`, `ollama`, `lmstudio` or `vllm`; empty uses your Kara configuration. User setting only. |
| `kara.model` | Model for editor sessions; empty uses your Kara configuration. User setting only. |
| `kara.binary.path` | Path to `kara`. Machine setting. |
| `kara.autoStart` | Start Kara when a folder opens. |
| `kara.includeDiagnostics` | Send diagnostics with selection commands. |

Security-sensitive settings cannot be set from a workspace's
`.vscode/settings.json`, so a repository cannot enable `full` mode, redirect
inference or swap the binary.

## Commands

Kara: Open · New Session · Ask About Selection · Fix Selection · Review
Current Diff · Run Tests · Choose Model · Connect to Inference Machine ·
Doctor · Show Changes · Undo Last Change Batch · Stop Current Task · Restart
Agent

## Links

Source, documentation and issues: https://github.com/iamzayn19/kara

License: Apache-2.0
