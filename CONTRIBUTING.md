# Contributing

Thanks for helping. Veyra aims to be dependable infrastructure: structured
tools, real tests, honest behavior. Contributions that keep it that way are
very welcome.

## Before you start

* For anything beyond a small fix, open an issue first to agree on the
  approach.
* Read [ARCHITECTURE.md](ARCHITECTURE.md) and [SECURITY.md](SECURITY.md).
  Changes to permissions, path handling, command classification or the
  journal need tests that demonstrate the boundary.

## Pull requests

* `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`
  and `cargo test --workspace` must pass. For extension changes,
  `npm test` in `extensions/vscode`.
* Add tests with behavior changes. Agent behavior is tested with the scripted
  provider against fixture repositories, not with live models.
* Keep the agent model-agnostic. Model-specific handling belongs in the
  registry or the provider, not in the loop.
* Keep the extension a UI. Agent logic lives in the Rust binary.
* No telemetry, analytics or network calls beyond what PRIVACY.md lists.
* Describe user-visible changes in CHANGELOG.md under "Unreleased".

## Good first contributions

* Language pack improvements (better symbol patterns, test commands for more
  setups) with a fixture.
* New fixture tasks for the evaluation suite.
* Test output parsers for more frameworks (`crates/veyra-tools/src/testparse.rs`).
* Evaluation results for models on your hardware (see
  docs/MODEL_EVALUATION.md).

## License

By contributing you agree that your contributions are licensed under the
Apache License 2.0, the project's license.
