# Development

## Requirements

* Rust (stable; `rust-toolchain.toml` pins the channel and components)
* git
* For the VS Code extension: Node.js 20+ (24 recommended for running the
  TypeScript fixture)
* Optional, for fixture checks: Python 3, Ruby, Go, a JDK and make

## Build and test

```sh
cargo build                      # debug build of `kara` (target/debug/kara)
cargo test --workspace           # unit + integration tests (mocked models)
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Integration tests in `tests/integration/` copy fixture repositories into
temporary git repos and drive the full agent loop with a scripted model.
Fixture tasks whose toolchain is missing are skipped, not failed.

```sh
cd extensions/vscode
npm ci
npm test                                  # compile + unit tests
KARA_BIN=../../target/debug/kara npm test   # + protocol test against the binary
KARA_TEST_MODEL=1 KARA_BIN=... npm test     # + a prompt against your local model
npm run package                           # produces kara-<version>.vsix
```

## Evaluation

```sh
target/debug/kara eval --oracle             # validates fixtures and harness, no model
target/debug/kara eval --model qwen3-4b-q4_k_m --yes
target/debug/kara eval --tasks 'python-*'
```

Real-model evaluations are slow and run manually or before a release, never
in every CI job. Results go to `tests/evals/results/`. Record summaries in
`docs/MODEL_EVALUATION.md`.

## Isolated state

Set `KARA_HOME` to keep experiments away from your real Kara config and data:

```sh
KARA_HOME=/tmp/kara-dev target/debug/kara doctor
```

## Layout

```
crates/          Rust crates (see ARCHITECTURE.md)
extensions/vscode  VS Code extension (TypeScript, UI only)
languages/       language packs (TOML, compiled in)
models/          model registry and pinned llama.cpp runtime (TOML, compiled in)
tests/fixtures/repos  fixture repositories with intentional bugs and reference solutions
tests/integration     end-to-end tests
tests/evals           evaluation results
training/        optional, offline fine-tuning tooling (not needed to run Kara)
scripts/         install, release and maintenance scripts
docs/            protocol, configuration, evaluation, release checklist
```

## Adding a fixture task

1. Add or extend a repo in `tests/fixtures/repos/<name>/` with a failing test.
2. Describe the task in `kara-tasks.toml` (prompt, check command,
   expected changed files).
3. Add the reference fix as `solutions/<task-id>.patch` (unified diff).
4. `cargo test -p kara-integration-tests --test fixtures` verifies that the
   check fails before and passes after the patch.

## Updating pinned downloads

* llama.cpp: `scripts/update-llama-pin.sh <tag>` prints a new
  `models/runtimes.toml` from the GitHub release API (digests included).
* Models: `scripts/model-info.sh <repo> <file>` prints size, revision and
  SHA-256 from the Hugging Face API for `models/registry.toml`.
