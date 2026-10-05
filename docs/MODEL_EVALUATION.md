# Model evaluation

Kara picks default models by measured agent performance, not marketing
claims or parameter counts. This document describes the harness and records
results. **A model is not called "best" here without results to back it.**

## Harness

`kara eval` runs every task in `tests/fixtures/repos/*/kara-tasks.toml`:

1. Copy the fixture into a fresh temporary git repository (applying any
   prerequisite reference solutions listed in `requires`).
2. Run one agent turn with the task prompt. Permission mode: full; hard
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

`kara eval --oracle` replays the reference solutions through the real tools.
It must solve every task; CI runs it to keep fixtures and the harness honest.
It says nothing about any model.

## Running

```sh
kara eval --model qwen3-4b-q4_k_m --yes          # downloads the model if needed (asks without --yes)
kara eval --model qwen3.6-35b-a3b-q4_k_m --tasks 'rust-*,python-*'
kara eval --oracle
```

Each task has a wall-clock budget (`--task-timeout`, default 1800 s) so a
benchmark run always ends. This bounds the evaluation, not the agent:
interactive Kara has no step or time limit.

Reports are written to `tests/evals/results/<date>-<model>.json`. Real-model
evaluations are slow and run manually, never in every CI job.

## Results

Results are machine-specific. Speed depends on hardware; solve rate depends
on the model, quantization and Kara version.

<!-- RESULTS:BEGIN -->
### Qwen3-4B Q4_K_M · Apple M5, 16 GB (Metal) · 2026-10-05 · Kara 0.1.0

Full run with the first build: solved **5 / 10** runnable tasks (Go tasks skipped: Go not installed on this
machine). Generation speed about 12-15 tokens/s; every task took 3-10 minutes.
Per-task limit: 600 s.

| Task | Category | Solved | Outcome | Model calls | Tool calls | Time (s) | Tokens in/out | Peak mem |
|---|---|---|---|---|---|---|---|---|
| java-order-total | multi-file-bug | yes | completed | 7 | 6 | 311 | 27918/4177 | 6.9 GB |
| python-discount | one-line-fix | yes | completed | 5 | 4 | 267 | 19716/3814 | 7.6 GB |
| python-validation | missing-validation | no | completed | 6 | 4 | 304 | 23387/4571 | 7.6 GB |
| python-import | broken-import | no | time limit | 11 | 10 | 601 | 40464/7946 | 7.7 GB |
| ruby-session-expiry | one-line-fix | no | completed | 7 | 5 | 411 | 27196/5848 | 7.7 GB |
| ruby-email-case | multi-file-bug | yes | completed | 5 | 4 | 183 | 18922/2430 | 7.7 GB |
| rust-ledger-debit | one-line-fix | yes | completed | 5 | 3 | 198 | 19752/2670 | 7.5 GB |
| rust-counter-race | concurrency | yes | completed | 5 | 3 | 222 | 19589/3035 | 7.6 GB |
| ts-pagination-offset | one-line-fix | no | completed | 7 | 5 | 424 | 30514/5745 | 7.6 GB |
| ts-query-validation | missing-validation | no | time limit | 7 | 6 | 603 | 24771/6217 | 7.9 GB |

No invalid tool calls and no unneeded edits in any task.

What the failures taught us:

* In four failures the model declared success while its own last test run
  was still failing. Kara now pushes back when that happens (up to twice),
  and if the model still stops, appends a factual "tests are still failing"
  note to the answer instead of letting a false success stand.
* In `python-import` the model edited the test file instead of the code.
  The system prompt now forbids weakening tests unless asked.
* In `python-validation` the fix dropped `add_item`'s return value. This is
  a plain capability limit of a 4B model.

Raw report: `tests/evals/results/2026-10-05-qwen3-4b-q4-k-m.json`.

#### Rerun of the five failed tasks after the "still failing" gate

Same model, machine and limits, with the updated agent (push back when tests
still fail, no test editing):

| Task | Before | After | Model calls | Time (s) |
|---|---|---|---|---|
| python-validation | no | **yes** | 4 | 340 |
| python-import | no (time limit) | no (time limit) | 6 | 601 |
| ruby-session-expiry | no | **yes** | 5 | 199 |
| ts-pagination-offset | no | **yes** | 9 | 265 |
| ts-query-validation | no (time limit) | no (time limit) | 16 | 602 |

3 of the 5 previous failures are now solved. This is a rerun of the failed
subset, not a fresh full run. Local models are not deterministic, so the full
suite should be rerun before quoting an overall rate for this build. Raw
report: `tests/evals/results/2026-10-05-qwen3-4b-q4-k-m-rerun.json`.

This is one small model on one modest machine. It shows Kara's loop works
end to end with a real local model, not how capable the high-tier default is.
No `eval_score` has been recorded in the registry from this run: scores
are only comparable when the same suite was run on the same Kara version,
and the larger models have not been measured yet.
<!-- RESULTS:END -->

## Selection policy

Until measured results exist for a model, `/model auto` uses the provisional
`quality_rank` in `models/registry.toml`, which follows the project's initial
tiering (Qwen3.6-35B-A3B high, Qwen3.6-27B mid, smaller models below). Once a
model has results on representative hardware, set `eval_score` (fraction
solved) in the registry. Measured scores are used only when every model that
fits a machine has one; otherwise all candidates are ranked by
`quality_rank`, so one measured model can never outrank unmeasured ones.

Results still missing before the high-tier default can be called validated:
Qwen3.6-35B-A3B and Qwen3.6-27B need a machine with at least 24-32 GB of fast
memory. Contributions of results from such hardware are welcome.
