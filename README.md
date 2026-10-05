# Kara

**Kara runs on your machine. Intelligence runs wherever your compute is.**

Kara is a local-first coding agent with a native Rust core, a terminal
interface and a thin VS Code extension. Describe a task in plain language;
Kara searches the repository, reads the relevant code, edits it, runs the
tests, recovers from failures and reports what changed.

- One small native binary. No Node.js or Python runtime required.
- No account, no subscription, no Kara-hosted service, no telemetry.
- No message, token, daily or monthly quota. Kara does not meter usage.
- Inference runs where you choose: on this machine, on another machine you
  own, or on any OpenAI-compatible endpoint you control.

> **Status: 0.1.0, pre-release.** Kara works end to end with real models,
> but has not yet been used by people outside the project. See
> [docs/RELEASE_CHECKLIST.md](docs/RELEASE_CHECKLIST.md).

## Same Kara on every machine

Kara Core does not require a GPU, a minimum amount of RAM, or a local model.
It detects what the machine has (RAM, CPU architecture, accelerators, disk)
and adapts:

| Machine | What happens |
|---|---|
| 4 GB laptop | Kara runs fully as an agent. It recommends connecting to a machine you own (`kara connect`) or an endpoint such as Ollama, LM Studio or vLLM. |
| 16 GB laptop | Same Kara, plus an optional small local model (asked before any download). |
| Workstation with a large GPU | Same Kara, with a large local model, and it can share that inference with your other machines (`kara serve --inference`). |

No model is downloaded because Kara or the extension was installed. Local
inference starts only after you choose it.

## What it looks like

```
$ kara
Kara 0.1.0 · Kara runs on your machine. Intelligence runs wherever your compute is.
Inference:   Qwen3-4B Q4_K_M (local, 32k context)
Repository:  shop (git: main) · python
Permissions: workspace · /help for commands, Ctrl-C cancels a running task, Ctrl-D exits

› The cart discount tests are failing. Find the bug and fix it.
→ read shop/cart.py  ✓ shop/cart.py (1-30 of 30 lines)
→ edit shop/cart.py  ✓ shop/cart.py (+1 -1)
  ~ shop/cart.py (+1 −1)
      -        return round(self.total() * percent / 100)
      +        return round(self.total() * (1 - percent / 100))
→ test: python3 -m unittest test.test_cart  ✓ PASSED: 4 passed, 0 failed (0.1s)
  tests passed: 4 passed, 0 failed
The bug was in `apply_discount`: it returned the discount instead of the
discounted total. All four tests in test/test_cart.py pass.

Changed:
  shop/cart.py
```

That run used a 4B model on a 16 GB laptop. If the model claims success
while tests fail, or claims an edit that never applied, Kara pushes back and
reports the real state instead of the claim.

## Install

```sh
# macOS / Linux
curl -fsSL https://raw.githubusercontent.com/iamzayn19/kara/main/scripts/install.sh | sh

# Windows (PowerShell)
irm https://raw.githubusercontent.com/iamzayn19/kara/main/scripts/install.ps1 | iex

# From source (Rust stable)
cargo install --git https://github.com/iamzayn19/kara kara-cli
```

Installers verify the release's `SHA256SUMS`. Homebrew and winget packages
are generated with each release ([packaging/](packaging/README.md)).

## Choose where inference runs

```sh
kara doctor                                   # what this machine can do
kara models                                   # local models that fit, if any

# Local (optional): download a model after reviewing size, memory and license
kara models pull qwen3-4b-q4_k_m

# Another machine you own
#   on the machine with the GPU:
kara serve --inference --listen 0.0.0.0:7878  # prints a token
#   on this machine:
kara connect http://gpu-box:7878 --token <token>

# An OpenAI-compatible server you run
kara config set inference.provider ollama     # or lmstudio, vllm, openai_compat
kara config set inference.model qwen3:8b
```

See [INFERENCE.md](INFERENCE.md).

## Talk to it

```
› Explain how payments work in this repository.
› Fix the failing authentication specs.
› Add pagination to the users endpoint and test it.
› Review my current diff.
› Find the race condition in the request counter.
```

### Slash commands

| | |
|---|---|
| `/morpheus <task>` (`/plan`) | Investigate, explain the approach, list files and tests involved, and wait for `/approve` before changing anything. |
| `/oracle` (`/review`) | Risk review of the current diff by severity, with `path:line`. |
| `/matrix` | What Kara sees: repository stats, active files and symbols, your changes vs. Kara's, inference, context usage, tools, task. |
| `/diff` `/undo` `/git` | Inspect and roll back Kara's changes. Your own edits are never overwritten. |
| `/test` `/lint` `/build` | Run the project's detected commands. |
| `/model` `/models` `/model auto` | Show and switch inference. |
| `/permissions [mode]` | Show or change the permission mode for this session. |
| `/status` `/context` `/compact` `/clear` `/doctor` `/privacy` `/sessions` `/help` `/exit` | |

Non-interactive: `kara run "fix the failing tests"`, `kara run --plan ...`,
`kara run --review`, `kara undo`.

## Permissions

| Mode | Behaviour |
|---|---|
| `ask` | Reading, searching and tests run freely; every edit and shell command asks. |
| `workspace` (default) | Edits, tests, lint and builds inside the workspace run freely; risky or out-of-workspace actions ask. |
| `full` | Broad tool execution (shell, deletes, network, commits) without asking. |

In every mode, secrets, `sudo`, `git push`, destructive commands and paths
outside the workspace ask each time. `full` can only be enabled by you
(`kara config set permissions.mode full`, `--permissions full`, `/permissions full`
or a user-level editor setting), never by a repository's `.kara/config.toml`
or a workspace's `.vscode/settings.json`. See [SECURITY.md](SECURITY.md).

## VS Code

[`extensions/vscode`](extensions/vscode) is a thin client of the same Kara
binary: chat with streaming, tool activity, permission prompts, diff preview
with accept/reject, selection and diagnostics context, `/matrix`,
`/morpheus`, `/oracle`, `/model`, `/models` and `/status`. It uses `kara` from
PATH or downloads the release binary into Kara's data directory (checksums
verified). It never downloads a model on its own, and the CLI and editor
share one set of models.

## Files and configuration

| | Configuration | Data (models, cache, logs, sessions, managed binary) |
|---|---|---|
| macOS | `~/Library/Application Support/Kara/` | `~/Library/Application Support/Kara/` |
| Linux | `~/.config/kara/` | `~/.local/share/kara/` |
| Windows | `%APPDATA%\Kara\` | `%LOCALAPPDATA%\Kara\` |

```sh
kara config get permissions.mode
kara config set permissions.mode ask
kara config path
```

See [docs/CONFIGURATION.md](docs/CONFIGURATION.md).

## Languages

Kara works on any text repository. Language packs add symbol extraction and
test/lint/build discovery for Ruby, Python, JavaScript, TypeScript, Rust, Go,
Java, C, C++, C#, PHP, Swift, Kotlin, Shell, HTML/CSS and SQL
([LANGUAGE_SUPPORT.md](LANGUAGE_SUPPORT.md)).

## Notes

- Kara itself has no usage limits. Hardware, electricity and the licenses of
  the models you choose are yours to consider.
- Downloads come from third parties (GitHub, Hugging Face). Kara pins and
  verifies them, but cannot guarantee those services.
- Experimental in 0.1: Tree-sitter symbol extraction and LSP integration are
  planned; v0.1 uses pattern-based symbols and runs type checkers for
  diagnostics.

## Documentation

[ARCHITECTURE.md](ARCHITECTURE.md) · [INFERENCE.md](INFERENCE.md) ·
[SECURITY.md](SECURITY.md) · [PRIVACY.md](PRIVACY.md) ·
[LANGUAGE_SUPPORT.md](LANGUAGE_SUPPORT.md) · [DEVELOPMENT.md](DEVELOPMENT.md) ·
[CONTRIBUTING.md](CONTRIBUTING.md) · [CHANGELOG.md](CHANGELOG.md) ·
[Configuration](docs/CONFIGURATION.md) · [Protocol](docs/PROTOCOL.md)

## License

Apache-2.0. Copyright 2026 Muhammad Zain Ul Abidin (iamzayn19).
