# Editor protocol

`kara serve --stdio` speaks JSON-RPC 2.0. Each message is one JSON object on
one line (UTF-8, `\n`-terminated). Kara opens no network port for editors.
Diagnostics go to stderr. Protocol version: `1.0` (additive changes bump the
minor version).

Both sides send requests: the client drives the session, and Kara asks the
client for permission decisions.

## Client → Kara

| Method | Params | Result |
|---|---|---|
| `initialize` | `{workspace?, profile?, clientName?}` | `{protocolVersion, karaVersion, workspace, session, profile, model: {available, label, id, context}, commands: [{name, description}]}` |
| `session/prompt` | `{text, mode?: "execute"\|"plan"\|"review", context?, approvePlan?: bool}` | `TurnResult` when the turn ends. Events stream meanwhile. |
| `session/cancel` | `{}` (also accepted as a notification) | `{cancelled: true}` |
| `session/status` | `{}` | model, profile, pending plan, task state, context usage, index stats |
| `session/changes` | `{}` | `{kara: [{path, before, after, diff}], user: [paths]}`, meaning Kara's changes and the user's other uncommitted files |
| `session/undo` | `{}` | `{batch_id, restored, conflicts, errors}` |
| `session/new` | `{}` | `{session}` |
| `session/command` | `{name, args?}` with name `clear`, `compact`, `permissions`, `revert`, `privacy` or `matrix` | command-specific JSON |
| `doctor` | `{}` | hardware, memory budget, recommendation, llama.cpp location |
| `models/list` | `{}` | `{models: [{id, name, sizeBytes, license, source, revision, installed, fits, reason, memoryNeeded, recommended}], summary}` |
| `models/select` | `{id \| "auto", download: bool}` | the new model info. With `download: true` the client asserts that the user consented to the download. |
| `shutdown` | `{}` | `null`, then the process exits after stopping the model runtime |

`context` for prompts:

```json
{
  "file": "src/auth.ts",
  "selection": {"text": "...", "startLine": 10, "endLine": 24},
  "diagnostics": [{"file": "src/auth.ts", "line": 12, "severity": "error", "message": "..."}]
}
```

Only one prompt runs at a time. A second one gets error `-32001` (busy).
Prompts without a model get `-32002` (model unavailable).

## Kara → client

**Notifications**

* `event`: an `AgentEvent`, tagged by `type`:
  `turn_started`, `phase`, `assistant_delta`, `reasoning_delta`,
  `assistant_message`, `tool_started`, `tool_finished`, `file_changed`,
  `test_finished`, `plan_updated`, `notice`, `turn_finished`.
  The schema is defined in `crates/kara-protocol/src/lib.rs`.
* `log`: `{level, message}`, for diagnostics such as download progress.

**Requests**

* `permission/request`: a `PermissionRequest`
  `{id, tool, kinds, title, detail, reasons, can_remember}`. Reply with
  `{decision: "allow_once" | "allow_session" | "deny"}`. `allow_session` is
  downgraded to `allow_once` when `can_remember` is false (hard boundaries).
  If the client disconnects, the request is treated as denied.

## Example

```
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"workspace":"/home/me/app"}}
← {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"1.0","model":{"available":true,"label":"Qwen3-4B ..."}, ...}}
→ {"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{"text":"fix the failing tests"}}
← {"jsonrpc":"2.0","method":"event","params":{"type":"turn_started","turn_id":1,"mode":"execute","task":"fix the failing tests"}}
← {"jsonrpc":"2.0","method":"event","params":{"type":"tool_started","call_id":"c1","tool":"grep","summary":"grep `def test_`"}}
← {"jsonrpc":"2.0","id":1,"method":"permission/request","params":{"tool":"shell","kinds":["network"],"title":"run: pip install pytest",...}}
→ {"jsonrpc":"2.0","id":1,"result":{"decision":"deny"}}
...
← {"jsonrpc":"2.0","id":2,"result":{"turn_id":1,"outcome":"completed","summary":"...","changed_files":["app.py"], ...}}
```
