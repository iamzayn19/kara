# Configuration

Kara works without configuration. Settings live in TOML:

* `~/.kara/config.toml` (created on first run; `kara config path`)
* `<repository>/.kara/config.toml` (optional, may only make Kara stricter)
* `<repository>/.kara/instructions.md` (optional project conventions given to the model)

`kara config show` prints the effective configuration. Unknown keys are
rejected, so typos do not silently do nothing. Set `KARA_HOME` to move
`~/.kara`.

```toml
[model]
mode = "auto"              # auto | manual
id = ""                    # registry id for mode = "manual" (see `kara models`)
provider = "llamacpp"      # llamacpp | ollama | lmstudio | vllm | openai_compat
endpoint = ""              # for external providers; defaults per provider
api_model = ""             # model name at the endpoint
api_key_env = ""           # name of an env var holding an API key, if the server needs one
context_length = 0         # 0 = registry/auto-selected value
temperature = 0.2
reasoning = "auto"         # auto | on | off (thinking models)
reasoning_budget = 0       # thinking tokens per response; 0 = registry default, -1 = unlimited

[runtime]
llama_server_path = ""     # use this llama-server instead of PATH / managed install
bind_host = "127.0.0.1"    # loopback only unless allow_non_loopback = true
allow_non_loopback = false
gpu_layers = -1            # -1 = offload everything that fits
extra_args = []            # extra llama-server arguments
startup_timeout_secs = 300

[permissions]
profile = "balanced"       # safe | balanced | autonomous
allow_commands = []        # command prefixes you trust (ordinary shell only; user config only)
deny_commands = []         # command prefixes that are always refused
extra_readable_paths = []  # directories outside the workspace Kara may read (user config only)

[privacy]
telemetry = false          # Kara has no telemetry; true only prints a warning
training_data = false      # record sanitized traces locally in ~/.kara/traces (user config only)

[agent]
max_recovery_attempts = 8  # failed test runs after edits before Kara stops and reports
verify_after_edit = true   # ask the model to run tests before finishing after edits
show_reasoning = false     # stream thinking text in the terminal
repeat_guard = 3           # identical consecutive calls before Kara intervenes

[context]
max_orientation_files = 12
max_index_file_bytes = 1000000
tool_output_chars = 12000

[ui]
color = "auto"
```

There are no token, prompt, session or step quotas, and no setting to add
them. `max_recovery_attempts` and `repeat_guard` stop an unproductive loop
within one task; they do not limit how much you use Kara.

## What a repository config can change

Allowed: a stricter permission profile, additional `deny_commands`, and the
`[agent]` and `[context]` tuning sections.

Ignored with a warning: a looser profile, `allow_commands`,
`extra_readable_paths`, the model provider/endpoint/API key, any `[runtime]`
setting, and `privacy.training_data`.
