# Language support

Veyra is not tied to any language. Every repository gets the generic tools:
reading and searching files, editing, running commands, git and the model's
own reasoning. Language packs add better context and project-command
discovery on top.

## Enhanced language packs

| Pack | Extensions | Test discovery | Lint / typecheck | Build |
|---|---|---|---|---|
| Ruby | rb, rake, gemspec, ru, erb | `bundle exec rspec`, `bin/rails test`, `bundle exec rake test`, plain minitest | `bundle exec rubocop` | |
| Python | py, pyi | `pytest` (when configured), `unittest` | `ruff`, `flake8`, `mypy` | |
| JavaScript | js, jsx, mjs, cjs | package.json `test` script, `node --test` | `lint` script, `eslint` | `build` script |
| TypeScript | ts, tsx, mts, cts | package.json `test` script, `node --test` | `lint` script, `tsc --noEmit` | `build` script |
| Rust | rs | `cargo test` | `cargo clippy`, `cargo check` | `cargo build` |
| Go | go | `go test ./...` | `golangci-lint`, `go vet` | `go build ./...` |
| Java | java | Maven / Gradle (wrapper preferred) | | `mvn package`, `gradle build` |
| C | c, h | `ctest`, `make test` | | CMake, make |
| C++ | cpp, cc, cxx, hpp, ... | `ctest`, `make test` | | CMake, make |
| C# | cs | `dotnet test` | `dotnet format --verify-no-changes` | `dotnet build` |
| PHP | php | `vendor/bin/phpunit`, `composer test` | `phpstan` | |
| Swift | swift | `swift test` | `swiftlint` | `swift build` |
| Kotlin | kt, kts | Gradle | `ktlint` | Gradle |
| Shell | sh, bash, zsh, bats | `bats` | `shellcheck` | |
| HTML/CSS | html, css, scss, vue, svelte | | `stylelint` | |
| SQL | sql | | `sqlfluff` | |

Node projects use the package manager their lockfile indicates (npm, pnpm,
yarn or bun). Targeted test runs (`run_test` with files) use each pack's
`targeted` template, for example `bundle exec rspec spec/models/user_spec.rb`,
`go test ./pkg/auth` or `cargo test login`.

## How packs work

A pack is a TOML file in [`languages/`](languages/):

```toml
id = "ruby"
name = "Ruby"
extensions = ["rb", "rake"]
package_files = ["Gemfile", "*.gemspec"]
test_patterns = ["spec/**/*_spec.rb", "test/**/*_test.rb"]
lsp = ["ruby-lsp"]
tree_sitter = "ruby"

[[commands.test]]
run = "bundle exec rspec"
targeted = "bundle exec rspec {files}"
requires = ["Gemfile", "spec"]        # all must exist

[[commands.lint]]
run = "bundle exec rubocop"
requires = ["Gemfile", ".rubocop.yml"]

[[symbols]]
kind = "class"
pattern = '^\s*class\s+([A-Z][\w:]*)'

[imports]
patterns = ['''^\s*require(?:_relative)?\s+['"]([^'"]+)['"]''']
```

For each category the first command whose conditions hold is used.
Conditions: `requires` (all exist), `requires_any`, `requires_script` (a
package.json or composer.json script) and `requires_files_matching` (glob).
Placeholders in `targeted`: `{files}` (quoted paths), `{names}`/`{classes}`
(file stems), `{packages}` (`./dir`), `{modules}` (dotted Python modules).

Add or override packs without rebuilding by placing TOML files in
`~/.veyra/languages/`. A user pack with the same `id` replaces the built-in
one.

## Current limits (v0.1)

* **Symbols come from patterns, not parsers.** Definitions (classes,
  functions, methods, types, constants) are extracted by each pack's regular
  expressions. They are fast and robust, but can miss unusual layouts.
  Tree-sitter grammars are recorded per pack (`tree_sitter`) for a planned
  parser-based extractor. That backend is not shipped in v0.1.
* **LSP integration is experimental.** Packs name a language server, but v0.1
  does not start language servers. The `diagnostics` tool runs the project's
  type checker or linter and parses `file:line` output instead.
* **References are lexical.** `find_references` is a whole-word search that
  marks definitions found in the index. It does not resolve scopes or types.

If no test command is detected, tell Veyra how to run tests ("tests run with
`make check`") or add a pack rule.
