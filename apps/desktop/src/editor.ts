// Monaco setup, isolated from main.ts. Read-only viewer + diff editor only —
// Kara doesn't need IntelliSense/autocomplete for files it isn't editing
// directly, so this stays to the single generic editor.worker (no
// per-language workers), which is enough for syntax highlighting and the
// diff algorithm.

import * as monaco from "monaco-editor";

// Standard `new URL(...)` worker construction instead of Vite's `?worker`
// suffix import — this build's bundler (Rolldown) doesn't resolve that
// suffix the way classic Rollup-based Vite does, but every bundler
// understands this form.
(self as any).MonacoEnvironment = {
  getWorker() {
    return new Worker(new URL("./monaco.worker.ts", import.meta.url), { type: "module" });
  },
};

monaco.editor.defineTheme("kara-dark", {
  base: "vs-dark",
  inherit: true,
  rules: [],
  colors: {
    "editor.background": "#181819",
    "editor.lineHighlightBackground": "#1e1e2066",
    "editorLineNumber.foreground": "#55555a",
    "editorLineNumber.activeForeground": "#9a9aa0",
    "editorGutter.background": "#181819",
    "diffEditor.insertedTextBackground": "#1f332255",
    "diffEditor.removedTextBackground": "#3a222255",
  },
});

monaco.editor.defineTheme("kara-light", {
  base: "vs",
  inherit: true,
  rules: [],
  colors: {},
});

const EXT_TO_LANG: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  json: "json",
  py: "python",
  rs: "rust",
  go: "go",
  java: "java",
  rb: "ruby",
  php: "php",
  c: "c",
  h: "c",
  cpp: "cpp",
  hpp: "cpp",
  cs: "csharp",
  html: "html",
  css: "css",
  scss: "scss",
  md: "markdown",
  yml: "yaml",
  yaml: "yaml",
  toml: "ini",
  sh: "shell",
  sql: "sql",
};

export function languageForPath(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return EXT_TO_LANG[ext] ?? "plaintext";
}

export function monacoThemeFor(effective: "light" | "dark"): string {
  return effective === "light" ? "kara-light" : "kara-dark";
}

let viewer: monaco.editor.IStandaloneCodeEditor | undefined;

/** A single read-only file viewer, mounted once and reused across files
 * (disposing and recreating Monaco per file is unnecessary churn). */
export function showFile(container: HTMLElement, path: string, content: string, theme: string) {
  const language = languageForPath(path);
  if (!viewer) {
    viewer = monaco.editor.create(container, {
      value: content,
      language,
      theme,
      readOnly: true,
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 12.5,
      fontFamily: "ui-monospace, 'SF Mono', Menlo, monospace",
      scrollBeyondLastLine: false,
      renderLineHighlight: "none",
    });
  } else {
    const model = monaco.editor.createModel(content, language);
    const old = viewer.getModel();
    viewer.setModel(model);
    old?.dispose();
    monaco.editor.setTheme(theme);
  }
}

let diffViewer: monaco.editor.IStandaloneDiffEditor | undefined;

/** Side-by-side diff — Monaco's own, not a hand-rolled one. */
export function showDiff(container: HTMLElement, path: string, before: string, after: string, theme: string) {
  const language = languageForPath(path);
  const originalModel = monaco.editor.createModel(before, language);
  const modifiedModel = monaco.editor.createModel(after, language);
  if (!diffViewer) {
    diffViewer = monaco.editor.createDiffEditor(container, {
      theme,
      readOnly: true,
      automaticLayout: true,
      renderSideBySide: true,
      minimap: { enabled: false },
      fontSize: 12.5,
      fontFamily: "ui-monospace, 'SF Mono', Menlo, monospace",
    });
  } else {
    monaco.editor.setTheme(theme);
  }
  const old = diffViewer.getModel();
  diffViewer.setModel({ original: originalModel, modified: modifiedModel });
  old?.original.dispose();
  old?.modified.dispose();
}

export function setMonacoTheme(theme: string) {
  monaco.editor.setTheme(theme);
}

export function disposeDiff() {
  diffViewer?.dispose();
  diffViewer = undefined;
}
