# Privacy

Veyra runs locally. By default nothing about your code, prompts or usage
leaves your machine.

```
$ veyra privacy
Cloud inference:        disabled
Telemetry:              disabled
Account required:       no
Prompt uploads:         no
Repository uploads:     no
Model runtime:          local (llama.cpp, managed by Veyra)
Model endpoint:         127.0.0.1
Training collection:    disabled
```

The report is computed from your effective configuration, so it changes if you
point Veyra at a remote endpoint (then it says `REMOTE` and `Prompt uploads:
yes`).

## What uses the network

| When | What | Where |
|---|---|---|
| You approve installing the runtime | llama.cpp release archive | github.com (ggml-org/llama.cpp releases) |
| You approve a model download | GGUF model file | huggingface.co |
| The VS Code extension cannot find the binary and you approve | Veyra release archive and SHA256SUMS | github.com (iamzayn19/veyra releases) |
| You approve a command that needs it | whatever that command does (e.g. `npm install`) | as the command specifies |

Inference requests go only to the configured model endpoint, `127.0.0.1` by
default. Veyra contains no telemetry, crash reporting or analytics code. The
`privacy.telemetry` setting exists only to make that explicit; setting it to
`true` does nothing except print a warning.

## What is stored locally

```
~/.veyra/config.toml     your settings
~/.veyra/models/         downloaded models and their provenance records
~/.veyra/runtimes/       the llama.cpp build Veyra installed
~/.veyra/cache/          repository indexes (file paths and symbol names)
~/.veyra/sessions/       undo snapshots of files Veyra changed
~/.veyra/veyra.db        session history: your requests and Veyra's answers
~/.veyra/logs/           llama-server log
~/.veyra/history         terminal input history
```

Delete any of these at any time. Veyra never writes session data into your
repository.

## Training data

`privacy.training_data = false` by default. If you set it to `true` in your
**user** config (a repository cannot enable it), Veyra appends sanitized turn
traces (task, tool names and arguments, outcomes, test results, with credential
patterns redacted) to `~/.veyra/traces/`. These files stay on your machine.
Veyra never uploads them. See `training/README.md` for the optional, offline
fine-tuning tools that can use them.
