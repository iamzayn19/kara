# Model evaluation

Veyra picks default models by measured agent performance, not marketing
claims or parameter counts. This document describes the harness and records
results. **A model is not called "best" here without results to back it.**

## Harness

`veyra eval` runs every task in `tests/fixtures/repos/*/veyra-tasks.toml`:

1. Copy the fixture into a fresh temporary git repository (applying any
   prerequisite reference solutions listed in `requires`).
2. Run one agent turn with the task prompt. Profile: autonomous; hard
   boundaries (secrets, push, destructive, outside workspace) are always
   denied.
3. Run the task's check command independently of anything the agent
   reported. **Solved** means the check passes.
4. Record:

| Metric | Meaning |
|---|---|
| solved | the independent check command passes |
| outcome | completed / stalled / error |
| tests passing | the agent's own last test run passed |
| model calls | iterations of the loop |
| tool calls / tool errors / invalid calls | invalid = unknown tool or malformed arguments |
| unneeded edits | changed files outside the task's expected set (test files excluded) |
| time | wall clock per task |
| tokens in / out | as reported by the runtime |
| peak memory | resident memory of the llama-server process |

Tasks cover repository navigation and symbol discovery (every task starts
from a natural-language prompt), one-line fixes, bugs spread across files,
missing validation, broken imports, configuration bugs, a real data race,
tool calling, running tests and recovering from failing tests (tracked by
retries), and following instructions. Code review and code explanation are
exercised by `/oracle` and plan-mode tests but are not yet scored
automatically.

| Fixture | Language | Tasks |
|---|---|---|
| python-shop | Python | discount bug, validation, broken import |
| ruby-auth | Ruby (Rails-style) | session expiry, email normalization across files |
| typescript-pagination | TypeScript | off-by-one pagination, parameter validation |
| rust-ledger | Rust | debit sign bug, lost-update race condition |
| go-inventory | Go | reservation side effects and validation, env var config bug |
| java-orders | Java | order total and shipping threshold (two files) |

`veyra eval --oracle` replays the reference solutions through the real tools.
It must solve every task; CI runs it to keep fixtures and the harness honest.
It says nothing about any model.

## Running

```sh
veyra eval --model qwen3-4b-q4_k_m --yes          # downloads the model if needed (asks without --yes)
veyra eval --model qwen3.6-35b-a3b-q4_k_m --tasks 'rust-*,python-*'
veyra eval --oracle
```

Reports are written to `tests/evals/results/<date>-<model>.json`. Real-model
evaluations are slow and run manually, never in every CI job.

## Results

Results are machine-specific. Speed depends on hardware; solve rate depends
on the model, quantization and Veyra version.

<!-- RESULTS:BEGIN -->
_No results recorded yet._
<!-- RESULTS:END -->

## Selection policy

Until measured results exist for a model, `/model auto` uses the provisional
`quality_rank` in `models/registry.toml`, which follows the project's initial
tiering (Qwen3.6-35B-A3B high, Qwen3.6-27B mid, smaller models below). Once a
model has results on representative hardware, set `eval_score` (fraction
solved) in the registry. Automatic selection then prefers measured scores
over provisional ranks.

Results still missing before the high-tier default can be called validated:
Qwen3.6-35B-A3B and Qwen3.6-27B need a machine with at least 24-32 GB of fast
memory. Contributions of results from such hardware are welcome.
