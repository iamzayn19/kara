# Dependency license audit

Veyra is licensed under Apache-2.0. Every dependency must be distributable
under compatible terms. CI enforces the policy in `deny.toml` with
`cargo deny check`; run `scripts/license-audit.sh` locally.

## Rust dependencies (audited 2026-10-05, Veyra 0.1.0)

| License | Crates |
|---|---|
| MIT OR Apache-2.0 (and equivalent spellings) | 191 |
| MIT | 40 |
| Unicode-3.0 | 18 (ICU data used by URL handling) |
| Unlicense OR MIT | 7 (ripgrep components: ignore, globset, memchr, ...) |
| Apache-2.0 | 3 |
| Apache-2.0 OR ISC OR MIT / ISC / Apache-2.0 AND ISC | 5 (rustls, ring, webpki) |
| BSL-1.0, Apache-2.0 OR BSL-1.0 | 3 |
| BSD-3-Clause, Zlib, 0BSD variants | 4 |
| CDLA-Permissive-2.0 | 1 (webpki-roots: Mozilla root certificates) |
| MPL-2.0 | 1 (option-ext, via dirs; unmodified) |

No GPL, LGPL-only, AGPL, SSPL or proprietary dependencies. `r-efi` offers
LGPL-2.1-or-later as one option alongside MIT and Apache-2.0; Veyra uses it
under MIT/Apache-2.0.

## VS Code extension

The extension has no runtime npm dependencies. Only devDependencies
(TypeScript, type definitions, `@vscode/vsce`) are used to build it, and they
are not shipped in the VSIX.

## Downloaded at runtime (not distributed with Veyra)

| Component | License | When |
|---|---|---|
| llama.cpp prebuilt binaries | MIT | when you approve the runtime install |
| Model weights | per model (all registry models: Apache-2.0) | when you approve a model download |

The license of each model is shown before download and recorded in
`~/.veyra/models/.../veyra-model.json`.
