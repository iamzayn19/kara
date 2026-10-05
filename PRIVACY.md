# Privacy

Kara runs on your machine. Nothing about your code, prompts or usage leaves
it unless you point inference at another machine, and then only to that
machine.

```
$ kara privacy
Remote inference:       disabled
Telemetry:              disabled
Account required:       no
Prompt uploads:         no
Repository uploads:     no
Inference runs on:      this machine (local runtime managed by Kara)
Inference endpoint:     127.0.0.1
Training collection:    disabled
```

The report is computed from your effective configuration. With
`kara connect` or a non-local endpoint it says where inference runs and that
prompts (with code context) are sent there.

## What uses the network

| When | What | Where |
|---|---|---|
| You choose local inference and approve the runtime install | llama.cpp release archive | github.com (ggml-org/llama.cpp releases) |
| You approve a model download | model file | huggingface.co |
| The VS Code extension cannot find `kara` and you approve | Kara release archive and SHA256SUMS | github.com (iamzayn19/kara releases) |
| You configure another Kara machine or an endpoint | inference requests (prompts with code context) | the machine or endpoint you configured |
| You approve a command that needs it | whatever that command does (e.g. `npm install`) | as the command specifies |

Kara contains no telemetry, crash reporting or analytics code, and there is
no Kara-hosted service to send anything to. The `privacy.telemetry` setting
exists only to make that explicit; setting it to `true` does nothing except
print a warning.

## What is stored locally

In Kara's data directory (`~/Library/Application Support/Kara` on macOS,
`~/.local/share/kara` on Linux, `%LOCALAPPDATA%\Kara` on Windows):

```
models/      downloaded models and their provenance records (shared by CLI and editor)
runtimes/    the local runtime Kara installed, if any
bin/         the kara binary managed by the VS Code extension, if any
cache/       repository indexes (file paths and symbol names)
sessions/    undo snapshots of files Kara changed
logs/        local runtime log
kara.db      session history: your requests and Kara's answers
history      terminal input history
```

In Kara's config directory (the same folder on macOS, `~/.config/kara` on
Linux, `%APPDATA%\Kara` on Windows): `config.toml`, and under `credentials/`
the tokens for `kara serve --inference` / `kara connect` (owner-only files).

Delete any of these at any time. Kara never writes session data into your
repository.

## Training data

`privacy.training_data = false` by default. If you set it to `true` in your
**user** config (a repository cannot enable it), Kara appends sanitized turn
traces (task, tool names and arguments, outcomes, test results, with
credential patterns redacted) to `traces/` in its data directory. These files
stay on your machine; Kara never uploads them. See `training/README.md` for
the optional, offline fine-tuning tools that can use them.
