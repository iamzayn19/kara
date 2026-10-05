# Configuration

Kara works without configuration. Settings are TOML, layered:

1. built-in defaults
2. your user config (`kara config path`)
3. `<repository>/.kara/config.toml` (optional; may only make Kara stricter)
4. per-session overrides: `--permissions`, `--provider`, `--model`,
   `/permissions`, `/model`, and the VS Code user settings

`<repository>/.kara/instructions.md` (optional) holds project conventions for
the agent.

## Locations

| | User config | Data (models, cache, logs, sessions, managed binary) |
|---|---|---|
| macOS | `~/Library/Application Support/Kara/config.toml` | `~/Library/Application Support/Kara/` |
| Linux | `$XDG_CONFIG_HOME/kara/config.toml` (`~/.config/kara/`) | `$XDG_DATA_HOME/kara/` (`~/.local/share/kara/`) |
| Windows | `%APPDATA%\Kara\config.toml` | `%LOCALAPPDATA%\Kara\` |

`KARA_HOME=<dir>` puts both in one directory (tests, portable installs). The
CLI and the VS Code extension use the same locations, so downloaded models
and the managed binary exist once.

## Commands

```sh
kara config get permissions.mode            # effective value
kara config set permissions.mode ask        # edits your user config in place, keeps comments
kara config set inference.provider ollama
kara config show                            # whole effective configuration
kara config path
```

`set` validates every change against the schema: unknown keys and invalid
values are rejected. `get` and `show` include the repository's
`.kara/config.toml`; per-session flags apply only to that session.

## Reference

```toml
[inference]
provider = "local"        # local | kara | openai_compat | ollama | lmstudio | vllm
model = "auto"            # local: "auto" or a registry id; endpoints: model name ("" = first served)
endpoint = ""             # for kara / openai_compat; presets have defaults
api_key_env = ""          # env var holding a bearer token
api_key_file = ""         # file holding a bearer token (set by `kara connect`)
context_length = 0        # 0 = provider/registry default
temperature = 0.2
reasoning = "auto"        # auto | on | off (thinking models)
reasoning_budget = 0      # thinking tokens per response; 0 = registry default, -1 = unlimited

[inference.local]         # only used by the optional local runtime
server_path = ""          # use this runtime server binary instead of PATH / managed install
bind_host = "127.0.0.1"   # loopback only unless allow_non_loopback = true
allow_non_loopback = false
gpu_layers = -1           # -1 = offload as many layers as fit
extra_args = []
startup_timeout_secs = 300

[permissions]
mode = "workspace"        # ask | workspace | full
allow_commands = []       # command prefixes you trust (ordinary shell only; user config only)
deny_commands = []        # command prefixes that are always refused
extra_readable_paths = [] # directories outside the workspace Kara may read (user config only)

[privacy]
telemetry = false         # Kara has no telemetry; true only prints a warning
training_data = false     # record sanitized traces locally (user config only)

[agent]
max_recovery_attempts = 8 # failed test runs after edits before Kara stops and reports
verify_after_edit = true  # ask the model to run tests before finishing after edits
show_reasoning = false    # stream thinking text in the terminal
repeat_guard = 3          # identical consecutive calls before Kara intervenes

[context]
max_orientation_files = 12
max_index_file_bytes = 1000000
tool_output_chars = 12000

[ui]
color = "auto"            # auto | always | never
```

There are no message, token, session, daily or monthly quotas, and no setting
to add them. `max_recovery_attempts` and `repeat_guard` stop an unproductive
loop within one task; they do not limit how much you use Kara.

## What a repository config can change

Allowed: a stricter permission mode, additional `deny_commands`, and the
`[agent]` and `[context]` tuning sections.

Ignored with a warning: a looser mode (a repository can never enable
`full`), `allow_commands`, `extra_readable_paths`, anything under
`[inference]` (where inference runs is user configuration only), and
`privacy.training_data`.

## VS Code settings

| Setting | Scope | Meaning |
|---|---|---|
| `kara.permissions.mode` | user only | Mode for editor sessions; empty uses your Kara config. |
| `kara.inference.provider` | user only | Provider for editor sessions; empty uses your Kara config. |
| `kara.model` | user only | Model for editor sessions; empty uses your Kara config. |
| `kara.binary.path` | machine | Path to `kara`; empty uses PATH, then the managed copy. |
| `kara.autoStart` | window | Start Kara when a folder opens. |
| `kara.includeDiagnostics` | any | Send the file's diagnostics with selection commands. |

Workspace `.vscode/settings.json` cannot set the user-only settings, and the
extension reads only user values for them.
