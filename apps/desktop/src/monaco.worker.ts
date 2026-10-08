// A local entry point for the worker, so the `new URL(...)` construction in
// editor.ts gets a relative path Vite's static analysis handles regardless
// of bundler (Rolldown, in this project's case, doesn't resolve a bare
// `monaco-editor/...` specifier directly inside `new URL()`). The actual
// worker code still comes from the package via a normal import.
// monaco-editor's package.json exports map is `"./*": "./esm/vs/*.js"` —
// no `esm/vs` prefix in the specifier itself, which is easy to get wrong
// copying examples written for bundlers that resolve subpaths more loosely.
import "monaco-editor/editor/editor.worker.js";
