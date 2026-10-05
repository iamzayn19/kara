#!/bin/sh
# Audit dependency licenses for the Rust workspace and the VS Code extension.
# Requires cargo-deny (cargo install cargo-deny --locked) and Node.js.
set -eu
cd "$(dirname "$0")/.."
echo "== Rust dependencies (cargo-deny, policy in deny.toml)"
cargo deny check licenses
echo "== VS Code extension (runtime dependencies shipped in the VSIX)"
cd extensions/vscode
node -e '
const pkg = require("./package.json");
const deps = Object.keys(pkg.dependencies || {});
if (deps.length === 0) { console.log("no runtime dependencies (only devDependencies, which are not shipped)"); process.exit(0); }
for (const d of deps) { const p = require(`./node_modules/${d}/package.json`); console.log(`${d}: ${p.license}`); }
'
