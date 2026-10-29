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
    "editor.background": "#0e0f14",
    "editor.lineHighlightBackground": "#171923aa",
    "editorLineNumber.foreground": "#4a4d5e",
    "editorLineNumber.activeForeground": "#8c8fa3",
    "editorGutter.background": "#0e0f14",
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

/** One model per open file, kept across tab switches so unsaved edits and
 * undo history survive clicking away and back. */
const fileModels = new Map<string, monaco.editor.ITextModel>();

export function showFile(
  container: HTMLElement,
  path: string,
  content: string,
  theme: string,
  opts: { line?: number; readOnly?: boolean; onChange?: () => void } = {},
) {
  let model = fileModels.get(path);
  if (!model) {
    model = monaco.editor.createModel(content, languageForPath(path));
    if (opts.onChange) model.onDidChangeContent(opts.onChange);
    fileModels.set(path, model);
  }
  if (!viewer) {
    viewer = monaco.editor.create(container, {
      model,
      theme,
      readOnly: !!opts.readOnly,
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 12.5,
      fontFamily: "ui-monospace, 'SF Mono', Menlo, monospace",
      scrollBeyondLastLine: false,
      renderLineHighlight: "none",
    });
  } else {
    viewer.setModel(model);
    viewer.updateOptions({ readOnly: !!opts.readOnly });
    monaco.editor.setTheme(theme);
  }
  if (opts.line) {
    viewer.revealLineInCenter(opts.line);
    viewer.setSelection(new monaco.Selection(opts.line, 1, opts.line, model.getLineMaxColumn(opts.line)));
  }
  if (!opts.readOnly) viewer.focus();
}

/** Replaces an open file's text after it changed on disk, keeping the
 * scroll position and cursor where they were if it's on screen. */
export function replaceFileContent(path: string, content: string) {
  const model = fileModels.get(path);
  if (!model || model.getValue() === content) return;
  const shown = viewer?.getModel() === model;
  const state = shown ? viewer!.saveViewState() : null;
  model.setValue(content);
  if (shown && state) viewer!.restoreViewState(state);
}

export function fileContent(path: string): string | undefined {
  return fileModels.get(path)?.getValue();
}

export function closeFileModel(path: string) {
  const model = fileModels.get(path);
  if (!model) return;
  if (viewer?.getModel() === model) viewer.setModel(null);
  model.dispose();
  fileModels.delete(path);
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
