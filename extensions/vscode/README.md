# Kara for VS Code

Your code. Your machine. Your AI.

Kara is a free, open-source AI engineering agent that runs locally. This
extension is its VS Code interface: a chat sidebar that streams what the agent
does, shows diffs you can review and reject, and asks before anything risky.

No account. No subscription. No code uploads. Inference runs on your machine.

## Features

* **Chat** in the Kara activity bar: describe a task in plain language.
  Watch tool activity, test results and file changes as they happen.
* **Diff preview and reject** for every file Kara changes. Your own edits are
  never overwritten.
* **Permission dialogs** for writes, shell commands, network access and other
  risky actions, depending on the permission profile.
* **Plan and review modes**: `/morpheus <task>` proposes a plan and waits for
  approval; `/oracle` reviews your current diff by severity.
* **Editor context**: *Kara: Ask About Selection* and *Kara: Fix Selection*
  send the selected code and the file's diagnostics.
* **Model management**: *Kara: Choose Model* shows what fits your hardware,
  the download size and license, and downloads only after you confirm.

## Commands

Kara: Open · New Session · Ask About Selection · Fix Selection · Review
Current Diff · Run Tests · Choose Model · Doctor · Show Changes · Undo Last
Change Batch · Stop Current Task · Restart Agent

## How it works

The extension contains no agent logic. It starts the local `kara` binary
(`kara serve --stdio`) and talks JSON-RPC over stdin and stdout. No network
port is opened. If `kara` is not on your PATH, the extension offers to
download the official release from GitHub and verifies it against the
release's SHA256SUMS.

## Settings

* `kara.binaryPath`: path to the kara binary (default: PATH, then the
  managed copy)
* `kara.permissionProfile`: `safe`, `balanced` or `autonomous` (default: your
  `~/.kara/config.toml`)
* `kara.includeDiagnostics`: send the current file's diagnostics with
  selection commands

## Links

Source, documentation and issues: https://github.com/iamzayn19/kara

License: Apache-2.0
