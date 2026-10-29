// Kara desktop: a thin window over `kara serve --stdio`. All agent logic
// lives in the Rust core; this file only renders events and sends prompts,
// mirroring the VS Code extension's chatView.ts for the same protocol.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getVersion } from "@tauri-apps/api/app";
import { ask, open as openDialog } from "@tauri-apps/plugin-dialog";
import { fuzzyFilter } from "./fuzzy";
import { joinWorkspacePath } from "./paths";
import { shortcutKey } from "./keys";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
// Dynamically imported on first use (see loadEditorModule below), not at
// the top level — Monaco is a multi-MB dependency that has no reason to
// load on every launch for someone who never opens a file.
type EditorModule = typeof import("./editor");
let editorModule: EditorModule | undefined;
async function loadEditorModule(): Promise<EditorModule> {
  if (!editorModule) editorModule = await import("./editor");
  return editorModule;
}

// Same lazy-load pattern for the terminal panel (xterm.js) — no reason to
// pay for it unless the user actually opens a shell.
type TerminalModule = typeof import("./terminal");
let terminalModule: TerminalModule | undefined;
async function loadTerminalModule(): Promise<TerminalModule> {
  if (!terminalModule) terminalModule = await import("./terminal");
  return terminalModule;
}

const transcript = document.querySelector<HTMLElement>("#transcript")!;
const emptyState = document.querySelector<HTMLElement>("#empty-state")!;
const emptyText = document.querySelector<HTMLElement>("#empty-text")!;
const emptyOpenBtn = document.querySelector<HTMLButtonElement>("#empty-open-btn")!;
const input = document.querySelector<HTMLTextAreaElement>("#input")!;
const sendBtn = document.querySelector<HTMLButtonElement>("#send")!;
const sessionState = document.querySelector<HTMLElement>("#session-state")!;
const modelPill = document.querySelector<HTMLButtonElement>("#model-pill")!;
const gitPill = document.querySelector<HTMLElement>("#git-pill")!;
const cursorPos = document.querySelector<HTMLElement>("#cursor-pos")!;
const permPill = document.querySelector<HTMLButtonElement>("#perm-pill")!;
const contextPill = document.querySelector<HTMLButtonElement>("#context-pill")!;
const btnNew = document.querySelector<HTMLButtonElement>("#btn-new")!;
const btnUndo = document.querySelector<HTMLButtonElement>("#btn-undo")!;
const btnSlash = document.querySelector<HTMLButtonElement>("#btn-slash")!;
const sessionTimer = document.querySelector<HTMLElement>("#session-timer")!;
const sidebarTabs = document.querySelectorAll<HTMLButtonElement>(".sidebar-tab");
const sidebarPanelProject = document.querySelector<HTMLElement>("#sidebar-panel-project")!;
const sidebarPanelFiles = document.querySelector<HTMLElement>("#sidebar-panel-files")!;
const fileTree = document.querySelector<HTMLElement>("#file-tree")!;
const sidebarPanelSearch = document.querySelector<HTMLElement>("#sidebar-panel-search")!;
const searchInput = document.querySelector<HTMLInputElement>("#search-input")!;
const searchCaseBtn = document.querySelector<HTMLButtonElement>("#search-case")!;
const searchRegexBtn = document.querySelector<HTMLButtonElement>("#search-regex")!;
const searchSummary = document.querySelector<HTMLElement>("#search-summary")!;
const searchResults = document.querySelector<HTMLElement>("#search-results")!;
const filesBadge = document.querySelector<HTMLElement>("#files-badge")!;
const editorTabsEl = document.querySelector<HTMLElement>("#editor-tabs")!;
const editorContainer = document.querySelector<HTMLElement>("#editor-container")!;
const terminalPanel = document.querySelector<HTMLElement>("#terminal-panel")!;
const terminalMount = document.querySelector<HTMLElement>("#terminal-mount")!;
const btnTerminal = document.querySelector<HTMLButtonElement>("#btn-terminal")!;
const btnTerminalClose = document.querySelector<HTMLButtonElement>("#btn-terminal-close")!;
const composerWrap = document.querySelector<HTMLElement>("#composer-wrap")!;
const btnProject = document.querySelector<HTMLButtonElement>("#btn-project")!;
const btnSettings = document.querySelector<HTMLButtonElement>("#btn-settings")!;
const btnTheme = document.querySelector<HTMLButtonElement>("#btn-theme")!;
const btnSidebar = document.querySelector<HTMLButtonElement>("#btn-sidebar")!;
const sidebar = document.querySelector<HTMLElement>("#sidebar")!;
const recentList = document.querySelector<HTMLElement>("#recent-list")!;
const sessionsList = document.querySelector<HTMLElement>("#sessions-list")!;
const projectName = document.querySelector<HTMLElement>("#project-name")!;
const cmdMenu = document.querySelector<HTMLUListElement>("#cmd-menu")!;
const overlay = document.querySelector<HTMLElement>("#overlay")!;
const panelTitle = document.querySelector<HTMLElement>("#panel-title")!;
const panelBody = document.querySelector<HTMLElement>("#panel-body")!;
const panelClose = document.querySelector<HTMLButtonElement>("#panel-close")!;

let busy = false;
let pendingPlan = false;
let spinnerTimer: number | undefined;
let spinnerRow: HTMLElement | undefined;
let spinnerFrame = 0;
let availableCommands: { name: string; description: string }[] = [];
/** `model.provider` from initialize: local, kara, ollama, lmstudio, vllm or openai_compat. */
let inferenceProvider = "local";
/** A diff asked for while a turn was running (session/changes is busy
 * then); opened when the turn finishes. */
let pendingDiffPath: string | undefined;
/** karaVersion from initialize: the engine this window is talking to. */
let engineVersion = "";
/** Cached fs/files result; dropped when the project changes or Kara
 * reports changed files at the end of a turn. */
let projectFiles: string[] | undefined;
let cmdMenuIndex = 0;
let permissionsMode = "";
let turnMode: "execute" | "plan" | "review" = "execute";
const modeOpts = document.querySelectorAll<HTMLButtonElement>(".mode-opt");
const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
// Kara is named for Supergirl (Kara Zor-El) — the working indicator leans
// on that instead of a generic "thinking…", picked once per turn so it
// doesn't flicker between phrases every animation frame.
const WORKING_PHRASES = [
  "Flying in",
  "X-raying the code",
  "Super-speeding through it",
  "Listening in close",
  "Scanning the repo",
  "Channeling the House of El",
  "Warming up heat vision",
  "Orbiting the problem",
];
const LAST_WORKSPACE_KEY = "kara.lastWorkspace";
const RECENT_WORKSPACES_KEY = "kara.recentWorkspaces";
const MAX_RECENT = 8;

function recentWorkspaces(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_WORKSPACES_KEY);
    const list = raw ? JSON.parse(raw) : [];
    return Array.isArray(list) ? list.filter((p) => typeof p === "string") : [];
  } catch {
    return [];
  }
}

function addRecentWorkspace(path: string) {
  try {
    const list = [path, ...recentWorkspaces().filter((p) => p !== path)].slice(0, MAX_RECENT);
    localStorage.setItem(RECENT_WORKSPACES_KEY, JSON.stringify(list));
  } catch {
    /* per-viewer convenience; fine to lose */
  }
}
const THEME_KEY = "kara.theme";

type Theme = "system" | "light" | "dark";

function currentTheme(): Theme {
  try {
    const v = localStorage.getItem(THEME_KEY);
    if (v === "light" || v === "dark" || v === "system") return v;
  } catch {
    /* ignore */
  }
  // Dark is Kara's actual identity, not "whatever the OS happens to be" —
  // a bright cream window was the single biggest complaint testing this.
  // System is still one click away in Settings for anyone who wants it.
  return "dark";
}

function setTheme(t: Theme) {
  if (t === "system") document.documentElement.removeAttribute("data-theme");
  else document.documentElement.setAttribute("data-theme", t);
  try {
    if (t === "system") localStorage.removeItem(THEME_KEY);
    else localStorage.setItem(THEME_KEY, t);
  } catch {
    /* a per-viewer convenience; fine to lose on a private window */
  }
}

/** light or dark, resolving "system" against the OS preference — what's
 * actually on screen right now, not just the stored setting. */
function effectiveTheme(): "light" | "dark" {
  const t = currentTheme();
  if (t !== "system") return t;
  return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

/** The toolbar's one-click toggle — always lands on an explicit light or
 * dark (never back to "system"); Settings still has System for anyone who
 * wants to follow the OS instead. */
/** CSS follows the theme by itself; the editor and terminal are themed
 * from script. Only touches modules already loaded — a theme change
 * shouldn't force-load a multi-MB editor bundle nobody opened. */
function applyThemeToWidgets() {
  if (editorModule) editorModule.setMonacoTheme(editorModule.monacoThemeFor(effectiveTheme()));
  terminalModule?.setTerminalTheme(effectiveTheme());
}

function toggleTheme() {
  setTheme(effectiveTheme() === "light" ? "dark" : "light");
  applyThemeToWidgets();
}

// With "System" chosen, follow the OS switching light/dark while running.
window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", () => {
  if (currentTheme() === "system") applyThemeToWidgets();
});

// ── Transcript rendering ──────────────────────────────────────────

function clearEmptyState() {
  emptyState.style.display = "none";
}

function scrollToBottom() {
  transcript.scrollTop = transcript.scrollHeight;
}

const rawBubbleText = new WeakMap<HTMLElement, string>();

function bubble(text: string, who: "user" | "assistant"): HTMLElement {
  clearEmptyState();
  const row = document.createElement("div");
  row.className = `bubble-row from-${who}`;
  const b = document.createElement("div");
  b.className = `bubble ${who}`;
  if (who === "assistant") setBubbleMarkdown(b, text);
  else b.textContent = text;
  row.appendChild(b);
  if (who === "assistant") row.appendChild(copyButton(b));
  transcript.appendChild(row);
  scrollToBottom();
  return b;
}

/** Re-renders an assistant bubble from its raw (un-rendered) text — called
 * on every streamed delta, so this stays cheap: regex passes over a string
 * that's at most one message long, not a real parser. */
function setBubbleMarkdown(el: HTMLElement, raw: string) {
  rawBubbleText.set(el, raw);
  el.innerHTML = mdToHtml(raw);
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** Minimal, safe markdown: fenced + inline code, bold, italic, paragraphs.
 * Not a full CommonMark parser — assistant replies don't need tables or
 * nested lists to be dramatically more readable than one flat text blob. */
function mdToHtml(src: string): string {
  const blocks: string[] = [];
  const stash = (html: string): string => {
    const idx = blocks.push(html) - 1;
    return `\u0000${idx}\u0000`;
  };

  let text = src.replace(/```(\w*)\n([\s\S]*?)```/g, (_m, _lang, code) =>
    stash(`<pre class="md-code"><code>${escapeHtml(code.replace(/\n$/, ""))}</code></pre>`),
  );
  // An unterminated fence (still streaming): render what's there so far
  // rather than waiting for the closing ``` to show anything.
  text = text.replace(/```(\w*)\n([\s\S]*)$/, (_m, _lang, code) =>
    stash(`<pre class="md-code"><code>${escapeHtml(code)}</code></pre>`),
  );

  text = escapeHtml(text);
  text = text.replace(/`([^`\n]+)`/g, (_m, code) => stash(`<code class="md-inline-code">${code}</code>`));
  text = text.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
  text = text.replace(/(?<!\*)\*([^*\n]+)\*(?!\*)/g, "<em>$1</em>");

  text = text
    .split(/\n{2,}/)
    .map((p) => `<p>${p.replace(/\n/g, "<br>")}</p>`)
    .join("");

  return text.replace(/\u0000(\d+)\u0000/g, (_m, i) => blocks[Number(i)]);
}

function copyButton(target: HTMLElement): HTMLElement {
  const btn = document.createElement("button");
  btn.className = "copy-btn";
  btn.title = "Copy";
  btn.textContent = "⧉";
  btn.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(rawBubbleText.get(target) ?? target.textContent ?? "");
      btn.textContent = "✓";
      window.setTimeout(() => (btn.textContent = "⧉"), 1200);
    } catch {
      /* clipboard denied; not worth surfacing an error for */
    }
  });
  return btn;
}

function activity(text: string, cls?: string): HTMLElement {
  clearEmptyState();
  const el = document.createElement("div");
  el.className = "activity" + (cls ? ` ${cls}` : "");
  el.textContent = text;
  transcript.appendChild(el);
  scrollToBottom();
  return el;
}

function diffStat(diff: string): [number, number] {
  let add = 0;
  let del = 0;
  for (const l of diff.split("\n")) {
    if (l.startsWith("+") && !l.startsWith("+++")) add++;
    else if (l.startsWith("-") && !l.startsWith("---")) del++;
  }
  return [add, del];
}

function diffBlock(diff: string): HTMLElement {
  const pre = document.createElement("div");
  pre.className = "diff-block";
  for (const l of diff.split("\n")) {
    if (l.startsWith("---") || l.startsWith("+++")) continue;
    const span = document.createElement("span");
    span.textContent = l + "\n";
    if (l.startsWith("+")) span.className = "diff-add";
    else if (l.startsWith("-")) span.className = "diff-del";
    else if (l.startsWith("@@")) span.className = "diff-hunk";
    pre.appendChild(span);
  }
  return pre;
}

function fileChangedRow(change: { path: string; kind: string; diff: string; batch_id: number }) {
  clearEmptyState();
  const [add, del] = diffStat(change.diff);
  const sym = change.kind === "created" ? "+" : change.kind === "deleted" ? "-" : "~";
  const big = add + del > 24;

  const row = document.createElement("div");
  row.className = "file-row";

  const summary = document.createElement("div");
  summary.className = "file-summary";
  const label = document.createElement("span");
  label.className = "activity dim";
  label.textContent = `  ${sym} ${change.path}  (+${add} −${del})`;
  summary.appendChild(label);

  const actions = document.createElement("span");
  actions.className = "file-actions";

  let shown = !big;
  let container: HTMLElement | undefined;
  const render = () => {
    container?.remove();
    container = undefined;
    if (shown && change.kind !== "deleted") {
      container = diffBlock(change.diff);
      row.appendChild(container);
    }
  };

  if (change.kind !== "deleted") {
    const openDiff = document.createElement("button");
    openDiff.className = "file-action";
    openDiff.textContent = "open diff";
    openDiff.title = "Open a real side-by-side diff";
    openDiff.addEventListener("click", () => void openDiffTab(change.path));
    actions.appendChild(openDiff);
  }

  if (big && change.kind !== "deleted") {
    const toggle = document.createElement("button");
    toggle.className = "file-action";
    toggle.textContent = "view diff";
    toggle.addEventListener("click", () => {
      shown = !shown;
      toggle.textContent = shown ? "hide diff" : "view diff";
      render();
    });
    actions.appendChild(toggle);
  }

  {
    const reject = document.createElement("button");
    reject.className = "file-action reject";
    reject.textContent = "reject";
    reject.title = "Revert this file to how it was before Kara's change";
    reject.addEventListener("click", async () => {
      reject.disabled = true;
      reject.textContent = "reverting…";
      try {
        await invoke("kara_request", { method: "session/command", params: { name: "revert", args: change.path } });
        row.classList.add("file-rejected");
        actions.innerHTML = "";
        const done = document.createElement("span");
        done.className = "file-action-done";
        done.textContent = "reverted";
        actions.appendChild(done);
      } catch (e: any) {
        reject.disabled = false;
        reject.textContent = "reject";
        activity(`error: ${e?.message ?? e}`, "err");
      }
    });
    actions.appendChild(reject);
  }

  summary.appendChild(actions);
  row.appendChild(summary);
  render();
  transcript.appendChild(row);
  scrollToBottom();
}

function clearTranscript() {
  transcript.querySelectorAll(".bubble-row, .activity, .diff-block").forEach((el) => el.remove());
  removeApproveButton();
  emptyState.style.display = "flex";
  emptyText.textContent = "Describe what you want. Kara reads your repo, makes the change, and shows you the diff.";
  emptyOpenBtn.classList.add("hidden");
}

let pendingAssistant: HTMLElement | undefined;
let approveRow: HTMLElement | undefined;

function removeApproveButton() {
  approveRow?.remove();
  approveRow = undefined;
}

function showApproveButton() {
  removeApproveButton();
  const row = document.createElement("div");
  row.className = "approve-row";
  const btn = document.createElement("button");
  btn.className = "approve-btn";
  btn.textContent = "Approve plan";
  const edit = document.createElement("span");
  edit.className = "approve-hint";
  edit.textContent = "or reply below to change it";
  btn.addEventListener("click", () => approvePlan());
  row.appendChild(btn);
  row.appendChild(edit);
  transcript.appendChild(row);
  scrollToBottom();
  approveRow = row;
}

async function approvePlan() {
  removeApproveButton();
  pendingPlan = false;
  bubble("Approved.", "user");
  setBusy(true);
  try {
    await invoke("kara_request", {
      method: "session/prompt",
      params: { text: "", mode: "execute", approvePlan: true },
    });
  } catch (e: any) {
    activity(`error: ${e?.message ?? e}`, "err");
  } finally {
    setBusy(false);
  }
}

function startSpinner() {
  stopSpinner();
  clearEmptyState();
  spinnerRow = document.createElement("div");
  spinnerRow.className = "spinner-row krypton";
  transcript.appendChild(spinnerRow);
  spinnerFrame = 0;
  const phrase = WORKING_PHRASES[Math.floor(Math.random() * WORKING_PHRASES.length)];
  const started = Date.now();
  spinnerTimer = window.setInterval(() => {
    spinnerFrame++;
    const secs = Math.round((Date.now() - started) / 1000);
    if (spinnerRow) {
      spinnerRow.textContent = `${SPINNER[spinnerFrame % SPINNER.length]} ${phrase}… ${secs}s`;
    }
    scrollToBottom();
  }, 100);
}

function stopSpinner() {
  if (spinnerTimer !== undefined) {
    window.clearInterval(spinnerTimer);
    spinnerTimer = undefined;
  }
  spinnerRow?.remove();
  spinnerRow = undefined;
}

function setBusy(b: boolean) {
  busy = b;
  sendBtn.textContent = b ? "■" : "↑";
  sendBtn.classList.toggle("stop", b);
  sendBtn.disabled = false;
  sessionState.textContent = b ? "working…" : pendingPlan ? "plan ready — reply or say \"approve\"" : "ready";
  if (b) startSpinner();
  else stopSpinner();
}

function handleEvent(e: any) {
  switch (e.type) {
    case "turn_started":
      pendingAssistant = undefined;
      removeApproveButton();
      break;
    case "phase":
      if (e.phase === "recover") activity("→ tests failed; investigating and retrying", "warn");
      break;
    case "assistant_delta":
      if (!pendingAssistant) pendingAssistant = bubble("", "assistant");
      setBubbleMarkdown(pendingAssistant, (rawBubbleText.get(pendingAssistant) ?? "") + e.text);
      scrollToBottom();
      break;
    case "assistant_message":
      pendingAssistant = undefined;
      break;
    case "tool_started":
      activity(`→ ${e.summary}`, "krypton");
      break;
    case "tool_finished": {
      const mark = e.ok ? "✓" : "✗";
      const time = e.duration_ms >= 1000 ? ` (${(e.duration_ms / 1000).toFixed(1)}s)` : "";
      activity(`  ${mark} ${e.summary}${time}`, e.ok ? "dim" : "err");
      break;
    }
    case "file_changed":
      fileChangedRow(e.change);
      void reloadOpenFiles([e.change.path]);
      break;
    case "test_finished": {
      const r = e.report;
      const counts = r.passed != null && r.failed != null ? `${r.passed} passed, ${r.failed} failed` : `exit ${r.exit_code ?? "?"}`;
      if (r.passed != null && r.failed === 0) activity(`  tests passed: ${counts}`, "ok");
      else activity(`  tests failed: ${counts}`, "err");
      break;
    }
    case "plan_updated":
      for (const s of e.steps) {
        const mark = s.status === "done" ? "[x]" : s.status === "in_progress" ? "[>]" : s.status === "skipped" ? "[-]" : "[ ]";
        activity(`  ${mark} ${s.title}`, "dim");
      }
      if (e.hypothesis) activity(`  hypothesis: ${e.hypothesis}`, "dim");
      break;
    case "notice":
      activity(`${e.level === "error" ? "✗" : e.level === "warning" ? "!" : "·"} ${e.message}`, e.level === "error" ? "err" : e.level === "warning" ? "warn" : "dim");
      break;
    case "turn_finished":
      pendingAssistant = undefined;
      if (pendingDiffPath) {
        const p = pendingDiffPath;
        pendingDiffPath = undefined;
        void openDiffTab(p);
      }
      void refreshGitPill();
      void refreshDiffTabs();
      if (e.changed_files?.length) {
        projectFiles = undefined;
        // Shell commands can change files without a file_changed event.
        void reloadOpenFiles(e.changed_files);
      }
      pendingPlan = e.outcome === "awaiting_approval";
      if (e.changed_files?.length) {
        activity("Changed:", "accent");
        for (const f of e.changed_files) activity(`  ${f}`, "dim");
      }
      if (e.outcome === "awaiting_approval") {
        activity("Plan ready.", "accent");
        showApproveButton();
      } else if (e.outcome === "cancelled") {
        activity("cancelled", "warn");
      } else if (e.outcome === "stalled") {
        activity("stopped without finishing", "warn");
      } else if (e.outcome === "error") {
        activity("stopped on an error", "err");
      }
      if (e.usage && e.usage.prompt_tokens + e.usage.completion_tokens > 0) {
        activity(`${e.usage.prompt_tokens} tokens in, ${e.usage.completion_tokens} out`, "dim");
      }
      void updateContextPill();
      void refreshTreeBadgesIfVisible();
      break;
  }
}

// ── Sending / commands ─────────────────────────────────────────────

async function runCommand(name: string, args: string) {
  bubble(`/${name}${args ? ` ${args}` : ""}`, "user");
  try {
    const r: any = await invoke("kara_request", { method: "session/command", params: { name, args } });
    if (typeof r?.text === "string") activity(r.text, "dim");
    if (Array.isArray(r?.rows)) {
      // /permissions table.
      for (const row of r.rows) activity(`  ${row.kind}: ${row.decision}`, "dim");
    }
  } catch (e: any) {
    activity(`error: ${e?.message ?? e}`, "err");
  }
}

async function send() {
  if (busy) {
    await cancel();
    return;
  }
  const text = input.value.trim();
  if (!text) return;
  hideCmdMenu();

  if (text.startsWith("/")) {
    const [cmd, ...rest] = text.slice(1).split(/\s+/);
    input.value = "";
    autosize();
    await runCommand(cmd, rest.join(" "));
    return;
  }

  input.value = "";
  autosize();
  bubble(text, "user");
  setBusy(true);
  try {
    await invoke("kara_request", {
      method: "session/prompt",
      params: {
        text,
        mode: turnMode,
        approvePlan: turnMode === "execute" && pendingPlan && /^(y|yes|approve|go|ok|proceed)$/i.test(text),
      },
    });
  } catch (e: any) {
    activity(`error: ${e?.message ?? e}`, "err");
  } finally {
    setBusy(false);
  }
}

async function cancel() {
  try {
    await invoke("kara_notify", { method: "session/cancel", params: {} });
  } catch {
    /* best effort */
  }
}

async function newSession() {
  if (busy) await cancel();
  try {
    await invoke("kara_request", { method: "session/new", params: {} });
    clearTranscript();
    pendingPlan = false;
    sessionState.textContent = "ready";
    void updateContextPill();
    void refreshSessionsSidebar();
  } catch (e: any) {
    activity(`error: ${e?.message ?? e}`, "err");
  }
}

async function undo() {
  try {
    const r: any = await invoke("kara_request", { method: "session/undo", params: {} });
    const parts: string[] = [];
    if (r.restored?.length) parts.push(`restored ${r.restored.join(", ")}`);
    if (r.conflicts?.length) parts.push(`kept (edited since): ${r.conflicts.join(", ")}`);
    activity(parts.length ? `Undo: ${parts.join("; ")}` : "Nothing to undo.", "dim");
  } catch (e: any) {
    const msg = String(e?.message ?? e);
    activity(msg === "busy" ? "Kara is still working; stop the turn (Esc) or wait for it to finish, then undo." : `error: ${msg}`, msg === "busy" ? "warn" : "err");
  }
}

// ── Slash-command autocomplete ─────────────────────────────────────

function hideCmdMenu() {
  cmdMenu.classList.add("hidden");
  cmdMenu.innerHTML = "";
}

function showCmdMenu(prefix: string) {
  const matches = availableCommands.filter((c) => c.name.startsWith(`/${prefix}`)).slice(0, 8);
  if (matches.length === 0) {
    hideCmdMenu();
    return;
  }
  cmdMenuIndex = Math.min(cmdMenuIndex, matches.length - 1);
  cmdMenu.innerHTML = "";
  matches.forEach((c, i) => {
    const li = document.createElement("li");
    li.className = i === cmdMenuIndex ? "active" : "";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = c.name;
    const desc = document.createElement("span");
    desc.className = "desc";
    desc.textContent = c.description;
    li.appendChild(name);
    li.appendChild(desc);
    li.addEventListener("mousedown", (ev) => {
      ev.preventDefault();
      input.value = `${c.name} `;
      autosize();
      hideCmdMenu();
      input.focus();
    });
    cmdMenu.appendChild(li);
  });
  cmdMenu.classList.remove("hidden");
  return matches;
}

function updateCmdMenu() {
  const v = input.value;
  if (v.startsWith("/") && !v.includes(" ")) {
    showCmdMenu(v.slice(1));
  } else {
    hideCmdMenu();
  }
}

// ── Overlay panel (model picker / sessions / settings) ─────────────

function openPanel(title: string) {
  panelTitle.textContent = title;
  panelBody.innerHTML = "";
  overlay.classList.remove("hidden");
}

function closePanel() {
  overlay.classList.add("hidden");
  panelBody.innerHTML = "";
  progressFill = undefined;
  progressText = undefined;
}

let progressFill: HTMLElement | undefined;
let progressText: HTMLElement | undefined;

function showDownloadProgress(label: string) {
  overlay.classList.remove("hidden");
  panelTitle.textContent = "Downloading";
  panelBody.innerHTML = "";
  const wrap = document.createElement("div");
  wrap.className = "progress-wrap";
  const text = document.createElement("div");
  text.className = "progress-label";
  text.textContent = label;
  const track = document.createElement("div");
  track.className = "progress-track";
  const fill = document.createElement("div");
  fill.className = "progress-fill";
  track.appendChild(fill);
  wrap.appendChild(text);
  wrap.appendChild(track);
  panelBody.appendChild(wrap);
  progressFill = fill;
  progressText = text;
}

/** Driven by `kara://log` events with level "progress" (see RpcUi::progress
 * in serve.rs) — structured done/total, not just a log line, so this can
 * render a real bar instead of spamming the transcript with one line per
 * chunk. */
function updateDownloadProgress(payload: any) {
  if (!progressFill || !progressText) return;
  const { label, done, total } = payload;
  if (total) {
    const pct = Math.min(100, Math.round((done / total) * 100));
    progressFill.style.width = `${pct}%`;
    progressText.textContent = `${label}: ${formatBytes(done)} / ${formatBytes(total)} (${pct}%)`;
  } else {
    progressText.textContent = `${label}: ${formatBytes(done)}`;
  }
}

panelClose.addEventListener("click", closePanel);
overlay.addEventListener("click", (e) => {
  if (e.target === overlay) closePanel();
});
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !overlay.classList.contains("hidden")) closePanel();
});

function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(0)} MB`;
  return `${n} B`;
}

function formatTokens(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

/** Refreshes the context-usage pill from session/status. Best-effort: a
 * failure here (e.g. between sessions) just leaves the pill as it was. */
async function updateContextPill() {
  try {
    const s: any = await invoke("kara_request", { method: "session/status", params: {} });
    const used = s.context?.historyTokens ?? 0;
    const window = s.context?.window ?? 0;
    if (!window) {
      contextPill.textContent = "";
      return;
    }
    const pct = Math.min(100, Math.round((used / window) * 100));
    contextPill.textContent = `${formatTokens(used)} / ${formatTokens(window)} ctx`;
    contextPill.className = "pill" + (pct >= 90 ? " warn" : "");
  } catch {
    /* not fatal — just skip the update */
  }
}

async function openModelPicker() {
  openPanel("Choose a model");
  const loading = document.createElement("div");
  loading.className = "panel-empty";
  loading.textContent = "Loading…";
  panelBody.appendChild(loading);
  try {
    const r: any = await invoke("kara_request", { method: "models/list", params: {} });
    panelBody.innerHTML = "";
    if (!r.models?.length) {
      const empty = document.createElement("div");
      empty.className = "panel-empty";
      empty.textContent = r.summary ?? "No models available.";
      panelBody.appendChild(empty);
      return;
    }
    const summary = document.createElement("div");
    summary.className = "panel-section-label";
    summary.textContent = r.summary ?? "";
    panelBody.appendChild(summary);
    for (const m of r.models) {
      const row = document.createElement("div");
      row.className = "panel-row";
      const left = document.createElement("div");
      const title = document.createElement("div");
      title.className = "title";
      title.textContent = `${m.recommended ? "★ " : ""}${m.name} ${m.quantization}`;
      const meta = document.createElement("div");
      meta.className = "meta";
      meta.textContent = `${formatBytes(m.sizeBytes)} · ${m.license}${m.installed ? " · installed" : ""}${!m.fits ? ` · ${m.reason}` : ""}`;
      left.appendChild(title);
      left.appendChild(meta);
      row.appendChild(left);
      row.addEventListener("click", async () => {
        if (!m.installed) {
          const ok = window.confirm(
            `Download ${m.name} ${m.quantization}?\n\nSize: ${formatBytes(m.sizeBytes)}\nMemory needed: about ${formatBytes(m.memoryNeeded ?? m.sizeBytes)}\nLicense: ${m.license}\n\nThe download is verified against a pinned SHA-256.`,
          );
          if (!ok) return;
        }
        showDownloadProgress(m.installed ? `Starting ${m.name}…` : `Downloading ${m.name} ${m.quantization}…`);
        modelPill.textContent = `loading ${m.name}…`;
        try {
          const sel: any = await invoke("kara_request", { method: "models/select", params: { id: m.id, download: true } });
          modelPill.textContent = sel?.available ? sel.label : "no model";
          modelPill.className = sel?.available ? "pill ready" : "pill warn";
          closePanel();
        } catch (e: any) {
          closePanel();
          activity(`error: ${e?.message ?? e}`, "err");
        }
      });
      panelBody.appendChild(row);
    }
  } catch (e: any) {
    panelBody.innerHTML = "";
    const err = document.createElement("div");
    err.className = "panel-empty";
    err.textContent = String(e?.message ?? e);
    panelBody.appendChild(err);
  }
}

async function refreshSessionsSidebar() {
  if (!currentWorkspace) {
    sessionsList.innerHTML = "";
    return;
  }
  try {
    const r: any = await invoke("kara_request", { method: "session/list", params: {} });
    sessionsList.innerHTML = "";
    if (!r.sessions?.length) {
      const empty = document.createElement("div");
      empty.className = "sidebar-empty";
      empty.textContent = "No past sessions yet.";
      sessionsList.appendChild(empty);
      return;
    }
    for (const s of r.sessions) {
      const row = document.createElement("button");
      row.className = "sidebar-row" + (s.id === r.current ? " current" : "");
      row.title = `${s.turns} turn${s.turns === 1 ? "" : "s"} · ${s.model}`;
      const title = document.createElement("span");
      title.className = "sidebar-row-title";
      title.textContent = s.title || "(empty)";
      row.appendChild(title);
      if (s.id !== r.current) {
        row.addEventListener("click", async () => {
          try {
            await invoke("kara_request", { method: "session/resume", params: { id: s.id } });
            clearTranscript();
            activity(`Resumed: ${s.title || s.id.slice(0, 8)}`, "dim");
            void updateContextPill();
            void refreshSessionsSidebar();
          } catch (e: any) {
            activity(`error: ${e?.message ?? e}`, "err");
          }
        });
      }
      sessionsList.appendChild(row);
    }
  } catch {
    sessionsList.innerHTML = "";
  }
}

async function openSettings() {
  openPanel("Settings");

  renderInferenceSettings();

  const appearanceLabel = document.createElement("div");
  appearanceLabel.className = "panel-section-label";
  appearanceLabel.textContent = "Appearance";
  panelBody.appendChild(appearanceLabel);
  const switcher = document.createElement("div");
  switcher.id = "theme-switch";
  for (const [value, label] of [["system", "System"], ["light", "Light"], ["dark", "Dark"]] as const) {
    const btn = document.createElement("button");
    btn.className = "theme-opt" + (currentTheme() === value ? " active" : "");
    btn.textContent = label;
    btn.addEventListener("click", () => {
      setTheme(value);
      applyThemeToWidgets();
      switcher.querySelectorAll(".theme-opt").forEach((b) => b.classList.remove("active"));
      btn.classList.add("active");
    });
    switcher.appendChild(btn);
  }
  panelBody.appendChild(switcher);

  const label = document.createElement("div");
  label.className = "panel-section-label";
  label.textContent = "Permissions";
  panelBody.appendChild(label);
  const options: [string, string, string][] = [
    ["ask", "Ask", "Confirm anything that reads outside the repo, runs a command, or changes files."],
    ["workspace", "Workspace", "Allow everything inside this repository without asking; still asks to leave it."],
    ["full", "Full", "Allow everything, including outside the repository. Highest risk."],
  ];
  for (const [mode, title, desc] of options) {
    const row = document.createElement("div");
    row.className = "perm-option" + (mode === permissionsMode ? " selected" : "");
    const left = document.createElement("div");
    const t = document.createElement("div");
    t.className = "title";
    t.textContent = title;
    const m = document.createElement("div");
    m.className = "meta";
    m.textContent = desc;
    left.appendChild(t);
    left.appendChild(m);
    row.appendChild(left);
    row.addEventListener("click", async () => {
      try {
        await invoke("kara_request", { method: "session/command", params: { name: "permissions", args: mode } });
        permissionsMode = mode;
        permPill.textContent = permPillLabel(mode);
        closePanel();
      } catch (e: any) {
        activity(`error: ${e?.message ?? e}`, "err");
      }
    });
    panelBody.appendChild(row);
  }

  const diagLabel = document.createElement("div");
  diagLabel.className = "panel-section-label";
  diagLabel.textContent = "Diagnostics";
  panelBody.appendChild(diagLabel);
  const diagBody = document.createElement("div");
  diagBody.className = "diagnostics";
  diagBody.textContent = "Loading…";
  panelBody.appendChild(diagBody);
  try {
    const d: any = await invoke("kara_request", { method: "doctor", params: {} });
    const hw = d.hardware ?? {};
    const rows: [string, string][] = [
      ["OS", `${hw.os_version ?? "?"} (${hw.arch ?? "?"})`],
      ["CPU", `${hw.cpu ?? "?"} · ${hw.cpu_cores ?? "?"} threads`],
      [
        "RAM",
        `${formatBytes(hw.total_ram ?? 0)} total, ${formatBytes(hw.available_ram ?? 0)} available`,
      ],
      ["GPU", (hw.gpus ?? []).map((g: any) => g.name).join(", ") || "none detected"],
      [
        "Accel",
        ["metal", "cuda", "vulkan", "rocm"].filter((k) => hw[k]).join(", ") || "none",
      ],
      ["Local runtime", d.localRuntime ?? "not installed"],
      ["Recommendation", d.recommendation?.summary ?? "—"],
    ];
    renderKeyValueRows(diagBody, rows);
  } catch (e: any) {
    diagBody.textContent = String(e?.message ?? e);
  }

  const aboutLabel = document.createElement("div");
  aboutLabel.className = "panel-section-label";
  aboutLabel.textContent = "About";
  panelBody.appendChild(aboutLabel);
  const aboutBody = document.createElement("div");
  aboutBody.className = "diagnostics";
  panelBody.appendChild(aboutBody);
  const appVersion = await getVersion().catch(() => "unknown");
  for (const [k, v] of [
    ["Kara desktop", appVersion],
    ["Kara engine", engineVersion || "not running (open a project)"],
  ]) {
    const row = document.createElement("div");
    row.className = "diag-row";
    const key = document.createElement("span");
    key.className = "diag-key";
    key.textContent = k;
    const val = document.createElement("span");
    val.className = "diag-val";
    val.textContent = v;
    row.append(key, val);
    aboutBody.appendChild(row);
  }  // For bug reports: where Kara's logs and config live.
  const folders = document.createElement("div");
  folders.className = "settings-form";
  aboutBody.appendChild(folders);
  invoke("kara_request", { method: "doctor", params: {} })
    .then((d: any) => {
      for (const [label, path] of [
        ["Show logs folder", d?.paths?.logs],
        ["Show config file", d?.paths?.config],
      ] as [string, string | undefined][]) {
        if (!path) continue;
        const b = document.createElement("button");
        b.className = "icon-text-btn";
        b.textContent = label;
        b.title = path;
        b.addEventListener("click", () => void revealItemInDir(path).catch((e) => activity(String(e), "err")));
        folders.appendChild(b);
      }
    })
    .catch(() => {});


  const privLabel = document.createElement("div");
  privLabel.className = "panel-section-label";
  privLabel.textContent = "Privacy";
  panelBody.appendChild(privLabel);
  const privBody = document.createElement("div");
  privBody.className = "diagnostics";
  privBody.textContent = "Loading…";
  panelBody.appendChild(privBody);
  try {
    const p: any = await invoke("kara_request", { method: "session/command", params: { name: "privacy", args: "" } });
    const rows: [string, string][] = [
      ["Inference", p.inference ?? "—"],
      ["Leaves this machine", p.remote_inference ? "yes, inference only" : "no"],
      ["Telemetry", p.telemetry ? "on" : "none"],
      ["Account required", p.account_required ? "yes" : "no"],
      ["Prompts uploaded", p.prompt_uploads ? "yes" : "no"],
      ["Repository uploaded", p.repository_uploads ? "yes" : "no"],
    ];
    renderKeyValueRows(privBody, rows);
    if (p.network_uses?.length) {
      const uses = document.createElement("div");
      uses.className = "diag-row";
      const label = document.createElement("span");
      label.className = "diag-key";
      label.textContent = "Network";
      const val = document.createElement("span");
      val.className = "diag-val";
      val.textContent = p.network_uses.join(", ");
      uses.appendChild(label);
      uses.appendChild(val);
      privBody.appendChild(uses);
    }
  } catch (e: any) {
    privBody.textContent = String(e?.message ?? e);
  }
}

function renderKeyValueRows(container: HTMLElement, rows: [string, string][]) {
  container.innerHTML = "";
  for (const [k, v] of rows) {
    const r = document.createElement("div");
    r.className = "diag-row";
    const kEl = document.createElement("span");
    kEl.className = "diag-key";
    kEl.textContent = k;
    const vEl = document.createElement("span");
    vEl.className = "diag-val";
    vEl.textContent = v;
    r.appendChild(kEl);
    r.appendChild(vEl);
    container.appendChild(r);
  }
}

contextPill.addEventListener("click", async () => {
  if (busy || !contextPill.textContent) return;
  await runCommand("compact", "");
  void updateContextPill();
});
modelPill.addEventListener("click", openModelPicker);
permPill.addEventListener("click", openSettings);
btnSettings.addEventListener("click", openSettings);
btnTheme.addEventListener("click", toggleTheme);

// ── Project (folder) picker ─────────────────────────────────────────

function shortPath(p: string): string {
  const parts = p.split(/[/\\]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : p;
}

let currentWorkspace = "";
let sessionTimerStart: number | null = null;
let sessionTimerInterval: number | undefined;

function formatElapsed(ms: number): string {
  const mins = Math.floor(ms / 60000);
  if (mins < 60) return `${mins}m`;
  return `${Math.floor(mins / 60)}h ${mins % 60}m`;
}

function startSessionTimer() {
  sessionTimerStart = Date.now();
  sessionTimer.textContent = "0m";
  if (sessionTimerInterval !== undefined) window.clearInterval(sessionTimerInterval);
  sessionTimerInterval = window.setInterval(() => {
    if (sessionTimerStart) sessionTimer.textContent = formatElapsed(Date.now() - sessionTimerStart);
  }, 30_000);
}

/** Toolbar/composer controls that need an active session — disabled
 * rather than left clickable-but-erroring when no project is open yet
 * (the model picker in particular used to just throw "call initialize
 * first" at the user). */
function permPillLabel(mode: string): string {
  switch (mode) {
    case "ask":
      return "⚠ Ask every time";
    case "workspace":
      return "Workspace";
    case "full":
      return "⚠ Full access";
    default:
      return mode || "—";
  }
}

function setProjectControlsEnabled(enabled: boolean) {
  for (const el of [modelPill, permPill, btnSettings, btnUndo, btnNew, btnSlash, input, sendBtn]) {
    el.toggleAttribute("disabled", !enabled);
  }
}

async function startWorkspace(workspace: string) {
  sessionState.textContent = "starting…";
  removeRestartButton();
  try {
    const info: any = await invoke("kara_start", { workspace });
    currentWorkspace = info?.workspace ?? workspace;
    inferenceProvider = info?.model?.provider ?? "local";
    engineVersion = info?.karaVersion ?? "";
    modelPill.textContent = info?.model?.available ? info.model.label : "no model";
    modelPill.className = info?.model?.available ? "pill ready" : "pill warn";
    // Why there's no model (memory too tight, nothing installed, endpoint
    // unreachable) and what to do about it, from Kara Core.
    const guidance: string | undefined = info?.model?.available ? undefined : info?.model?.guidance;
    modelPill.title = guidance ?? "Choose a model";
    if (guidance) {
      activity(guidance, "warn");
      activity("In this app: click the model pill to download one, or Settings → Where inference runs to use another machine or a model server.", "dim");
    }
    permissionsMode = info?.permissionsMode ?? "";
    permPill.textContent = permPillLabel(permissionsMode);
    availableCommands = info?.commands ?? [];
    projectName.textContent = info?.workspace ? shortPath(info.workspace) : "no project open";
    projectName.title = info?.workspace ?? "";
    // What the taskbar, Window menu and Alt-Tab show.
    const folder = info?.workspace?.split(/[\\/]/).filter(Boolean).pop();
    void getCurrentWindow()
      .setTitle(folder ? `${folder} — Kara` : "Kara")
      .catch(() => {});
    sessionState.textContent = "ready";
    setProjectControlsEnabled(true);
    void refreshGitPill();
    startSessionTimer();
    void updateContextPill();
    void refreshTreeBadgesIfVisible();
    try {
      localStorage.setItem(LAST_WORKSPACE_KEY, currentWorkspace);
    } catch {
      /* private window or storage disabled: just skip remembering it */
    }
    addRecentWorkspace(currentWorkspace);
    refreshRecentSidebar();
    void refreshSessionsSidebar();
  } catch (e: any) {
    activity(`Kara failed to start: ${e?.message ?? e}`, "err");
    sessionState.textContent = "not running";
    setProjectControlsEnabled(false);
    showRestartButton();
  }
}

let restartRow: HTMLElement | undefined;

function removeRestartButton() {
  restartRow?.remove();
  restartRow = undefined;
}

function showRestartButton() {
  removeRestartButton();
  const row = document.createElement("div");
  row.className = "approve-row";
  const btn = document.createElement("button");
  btn.className = "approve-btn";
  btn.textContent = "Restart Kara";
  btn.addEventListener("click", async () => {
    removeRestartButton();
    await startWorkspace(currentWorkspace);
  });
  row.appendChild(btn);
  transcript.appendChild(row);
  scrollToBottom();
  restartRow = row;
}

/** The native folder dialog directly, skipping the recents list — used by
 * the "choose a folder" action and the Cmd/Ctrl+O shortcut. */
async function pickProject() {
  // The home directory is only ever a starting point to browse from here
  // — never a workspace Kara indexes without the user picking something
  // inside it first.
  const defaultPath = await invoke<string | null>("kara_home_dir").catch(() => null);
  const dir = await openDialog({
    directory: true,
    multiple: false,
    title: "Open a project for Kara",
    defaultPath: defaultPath ?? undefined,
  });
  if (!dir || typeof dir !== "string") return;
  await openWorkspace(dir);
}

/** Asks before an action that would throw away unsaved editor tabs;
 * true when there are none or the user chose to discard them. */
async function confirmDiscardUnsaved(action: string): Promise<boolean> {
  const dirty = openTabs.filter((t) => t.dirty).map((t) => t.label);
  if (!dirty.length) return true;
  const list = dirty.length === 1 ? dirty[0] : `${dirty.length} files (${dirty.slice(0, 3).join(", ")}${dirty.length > 3 ? ", …" : ""})`;
  return ask(`${list} ${dirty.length === 1 ? "has" : "have"} unsaved changes. ${action} anyway and discard them?`, {
    title: "Unsaved changes",
    kind: "warning",
    okLabel: "Discard",
    cancelLabel: "Keep editing",
  });
}

async function openWorkspace(dir: string) {
  if (!(await confirmDiscardUnsaved("Switch projects"))) return;
  clearTranscript();
  pendingPlan = false;
  resetEditorTabs();
  await startWorkspace(dir);
  // A shell started in the old project would stay there.
  if (terminalModule) {
    await terminalModule.stopTerminal();
    if (!terminalPanel.classList.contains("hidden")) await terminalModule.openTerminal(terminalMount, currentWorkspace || dir, effectiveTheme());
  }
}

function resetEditorTabs() {
  for (const t of openTabs) if (t.kind === "file") editorModule?.closeFileModel(t.path);
  openTabs.length = 0;
  closedFiles.length = 0;
  activeTabId = "chat";
  renderEditorTabs();
  transcript.classList.remove("hidden");
  composerWrap.classList.remove("hidden");
  editorContainer.classList.add("hidden");
  fileTree.innerHTML = "";
  expandedDirs.clear();
  treeStale = false;
  changedPaths = new Map();
  filesBadge.textContent = "";
  filesBadge.classList.add("hidden");
  searchSeq++;
  searchResults.replaceChildren();
  searchSummary.textContent = "";
  projectFiles = undefined;
}

/** Clicking the project name shows recent projects first (VS Code's "Open
 * Recent" pattern) rather than jumping straight to a folder dialog every
 * time. */
/** Renders into the persistent sidebar, not a popup — recent projects
 * should be glanceable the way Cursor/VS Code's own sidebar is, not
 * hidden behind a click every time. */
function refreshRecentSidebar() {
  const recents = recentWorkspaces().filter((p) => p !== currentWorkspace);
  recentList.innerHTML = "";
  if (!recents.length) {
    const empty = document.createElement("div");
    empty.className = "sidebar-empty";
    empty.textContent = "No other recent projects.";
    recentList.appendChild(empty);
    return;
  }
  for (const path of recents) {
    const row = document.createElement("button");
    row.className = "sidebar-row";
    row.title = path;
    const title = document.createElement("span");
    title.className = "sidebar-row-title";
    title.textContent = shortPath(path);
    row.appendChild(title);
    row.addEventListener("click", () => openWorkspace(path));
    recentList.appendChild(row);
  }
}

btnProject.addEventListener("click", pickProject);
emptyOpenBtn.addEventListener("click", pickProject);

// ── Composer wiring ──────────────────────────────────────────────────

function autosize() {
  input.style.height = "auto";
  input.style.height = `${Math.min(input.scrollHeight, 160)}px`;
}

input.addEventListener("input", () => {
  autosize();
  updateCmdMenu();
});
input.addEventListener("keydown", (ev) => {
  const menuOpen = !cmdMenu.classList.contains("hidden");
  if (menuOpen && (ev.key === "ArrowDown" || ev.key === "ArrowUp")) {
    ev.preventDefault();
    cmdMenuIndex += ev.key === "ArrowDown" ? 1 : -1;
    updateCmdMenu();
    return;
  }
  if (menuOpen && ev.key === "Tab") {
    ev.preventDefault();
    const active = cmdMenu.querySelector("li.active .name");
    if (active?.textContent) {
      input.value = `${active.textContent} `;
      autosize();
      hideCmdMenu();
    }
    return;
  }
  if (ev.key === "Enter" && !ev.shiftKey && !ev.metaKey && !ev.ctrlKey) {
    ev.preventDefault();
    if (menuOpen) {
      const active = cmdMenu.querySelector("li.active .name");
      if (active?.textContent) {
        input.value = `${active.textContent} `;
        autosize();
        hideCmdMenu();
        return;
      }
    }
    send();
  } else if (ev.key === "Escape") {
    if (menuOpen) hideCmdMenu();
    else if (busy) cancel();
  }
});
sendBtn.addEventListener("click", send);
modeOpts.forEach((btn) => {
  btn.addEventListener("click", () => {
    turnMode = btn.dataset.mode as "execute" | "plan" | "review";
    modeOpts.forEach((b) => b.classList.toggle("active", b === btn));
    input.focus();
  });
});
btnNew.addEventListener("click", newSession);
btnUndo.addEventListener("click", undo);
btnSlash.addEventListener("click", () => {
  input.focus();
  if (!input.value.startsWith("/")) {
    input.value = "/" + input.value;
    autosize();
  }
  updateCmdMenu();
});

// Native-feeling shortcuts: Cmd on macOS, Ctrl elsewhere.
window.addEventListener("keydown", (e) => {
  const mod = e.metaKey || e.ctrlKey;
  if (!mod) return;
  const key = shortcutKey(e);
  // Ctrl+Tab / Ctrl+Shift+Tab cycle tabs (Ctrl on every platform, as in VS Code).
  if (e.key === "Tab" && e.ctrlKey) {
    e.preventDefault();
    const ids = ["chat", ...openTabs.map((t) => t.id)];
    const i = ids.indexOf(activeTabId);
    void activateTab(ids[(i + (e.shiftKey ? ids.length - 1 : 1)) % ids.length]);
    return;
  }
  if (e.shiftKey && key === "t") {
    e.preventDefault();
    const path = closedFiles.pop();
    if (path) void openFileTab(path);
    return;
  }
  if (key === "p") {
    e.preventDefault();
    void openPalette(e.shiftKey ? "commands" : "files");
    return;
  }
  if (e.shiftKey && key === "f") {
    e.preventDefault();
    if (sidebar.classList.contains("collapsed")) toggleSidebar();
    showSidebarPanel("search");
    return;
  }
  switch (key) {
    case "n":
      e.preventDefault();
      newSession();
      break;
    case ",":
      e.preventDefault();
      openSettings();
      break;
    case "o":
      e.preventDefault();
      pickProject();
      break;
    case "b":
      e.preventDefault();
      toggleSidebar();
      break;
    case "j":
      e.preventDefault();
      toggleTerminal();
      break;
    case "s":
      e.preventDefault();
      // Cmd/Ctrl+Alt+S saves every edited tab, as in VS Code.
      void (e.altKey ? saveAllTabs() : saveActiveTab());
      break;
    case "w":
      if (activeTabId !== "chat") {
        e.preventDefault();
        closeTab(activeTabId);
      }
      break;
  }
});

function toggleSidebar() {
  sidebar.classList.toggle("collapsed");
}
btnSidebar.addEventListener("click", toggleSidebar);

async function toggleTerminal() {
  const opening = terminalPanel.classList.contains("hidden");
  if (opening) {
    terminalPanel.classList.remove("hidden");
    const { openTerminal } = await loadTerminalModule();
    await openTerminal(terminalMount, currentWorkspace || (await invoke<string | null>("kara_home_dir")) || ".", effectiveTheme());
  } else {
    terminalPanel.classList.add("hidden");
  }
}

function closeTerminal() {
  terminalPanel.classList.add("hidden");
}

btnTerminal.addEventListener("click", toggleTerminal);
btnTerminalClose.addEventListener("click", closeTerminal);

window.addEventListener("resize", () => {
  if (!terminalPanel.classList.contains("hidden")) terminalModule?.fitTerminal();
});

// ── Boot ──────────────────────────────────────────────────────────

/** macOS gets an overlay titlebar (native traffic lights); reserve space
 * for them in the toolbar. No navigator.platform parsing library needed
 * for a yes/no check this simple. */
function markPlatform() {
  if (/Mac/.test(navigator.platform) || /Macintosh/.test(navigator.userAgent)) {
    document.body.classList.add("platform-macos");
  }
}

// Applied immediately at module load, not inside boot()'s first await, so
// there's no flash of the wrong theme before the RPC round-trip resolves.
setTheme(currentTheme());

// ── Sidebar tabs: Project (recent/sessions) vs Files (explorer) ─────

function showSidebarPanel(panel: string) {
  sidebarTabs.forEach((t) => t.classList.toggle("active", t.dataset.panel === panel));
  sidebarPanelProject.classList.toggle("hidden", panel !== "project");
  sidebarPanelFiles.classList.toggle("hidden", panel !== "files");
  sidebarPanelSearch.classList.toggle("hidden", panel !== "search");
  if (panel === "files" && !fileTree.hasChildNodes()) void loadFileTree();
  else if (panel === "files" && treeStale) void refreshFileTree();
  if (panel === "search") {
    searchInput.focus();
    searchInput.select();
  }
}

sidebarTabs.forEach((tab) => {
  tab.addEventListener("click", () => showSidebarPanel(tab.dataset.panel ?? "project"));
});

// ── Find in files ────────────────────────────────────────────────

let searchSeq = 0;
let searchTimer: number | undefined;

async function runSearch() {
  const query = searchInput.value;
  const seq = ++searchSeq;
  if (!query || !currentWorkspace) {
    searchResults.replaceChildren();
    searchSummary.textContent = "";
    return;
  }
  searchSummary.textContent = "Searching…";
  try {
    const r: any = await invoke("kara_request", {
      method: "fs/search",
      params: {
        query,
        caseSensitive: searchCaseBtn.classList.contains("active"),
        regex: searchRegexBtn.classList.contains("active"),
      },
    });
    if (seq !== searchSeq) return; // a newer keystroke already superseded this
    renderSearchResults(r);
  } catch (e: any) {
    if (seq !== searchSeq) return;
    searchResults.replaceChildren();
    searchSummary.textContent = String(e?.message ?? e);
  }
}

function renderSearchResults(r: { matches: { path: string; line: number; text: string }[]; files_with_matches: number; truncated: boolean }) {
  const byFile = new Map<string, { line: number; text: string }[]>();
  for (const m of r.matches) {
    if (!byFile.has(m.path)) byFile.set(m.path, []);
    byFile.get(m.path)!.push(m);
  }
  const n = r.matches.length;
  searchSummary.textContent = n
    ? `${n}${r.truncated ? "+" : ""} result${n === 1 ? "" : "s"} in ${byFile.size} file${byFile.size === 1 ? "" : "s"}`
    : "No results";
  const frag = document.createDocumentFragment();
  for (const [path, hits] of byFile) {
    const header = document.createElement("div");
    header.className = "search-file";
    header.title = path;
    const name = document.createElement("span");
    name.className = "search-file-name";
    name.textContent = path.split("/").pop() ?? path;
    const dir = document.createElement("span");
    dir.className = "search-file-dir";
    dir.textContent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
    const count = document.createElement("span");
    count.className = "search-file-count";
    count.textContent = String(hits.length);
    header.append(name, dir, count);
    frag.appendChild(header);
    for (const hit of hits) {
      const row = document.createElement("button");
      row.className = "search-hit";
      row.title = `${path}:${hit.line}`;
      const ln = document.createElement("span");
      ln.className = "search-hit-line";
      ln.textContent = String(hit.line);
      const text = document.createElement("span");
      text.className = "search-hit-text";
      text.textContent = hit.text.trim();
      row.append(ln, text);
      row.addEventListener("click", () => void openFileTab(path, hit.line));
      frag.appendChild(row);
    }
  }
  searchResults.replaceChildren(frag);
}

function scheduleSearch() {
  window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(runSearch, 200);
}

searchInput.addEventListener("input", scheduleSearch);
searchInput.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    window.clearTimeout(searchTimer);
    void runSearch();
  }
});
for (const btn of [searchCaseBtn, searchRegexBtn]) {
  btn.addEventListener("click", () => {
    btn.classList.toggle("active");
    void runSearch();
  });
}

// ── File explorer ────────────────────────────────────────────────

const treeIcon = (isDir: boolean, expanded = false) => (isDir ? (expanded ? "▾" : "▸") : "·");

/** path -> "M" (modified) | "+" (new), from session/changes — the same
 * data the "open diff" action already uses. Refreshed once per tree load;
 * stale between turns until the tree is reopened, which is an acceptable
 * trade for not re-walking the tree on every turn_finished. */
let changedPaths = new Map<string, string>();

async function loadChangedPaths() {
  changedPaths = new Map();
  try {
    const r: any = await invoke("kara_request", { method: "session/changes", params: {} });
    for (const f of r.kara ?? []) {
      changedPaths.set(f.path, f.before ? "M" : "+");
    }
  } catch {
    /* no active session yet, or nothing changed — badges just stay empty */
  }
}

async function loadFileTree() {
  fileTree.innerHTML = "";
  if (!currentWorkspace) {
    const empty = document.createElement("div");
    empty.className = "sidebar-empty";
    empty.textContent = "Open a project to browse its files.";
    fileTree.appendChild(empty);
    return;
  }
  const loading = document.createElement("div");
  loading.className = "sidebar-empty";
  loading.textContent = "Loading…";
  fileTree.appendChild(loading);
  try {
    const [entries] = await Promise.all([fsListDir(""), loadChangedPaths()]);
    const next = document.createElement("div");
    await renderTreeLevel(next, entries, 0);
    fileTree.replaceChildren(...next.childNodes);
    treeStale = false;
  } catch (e: any) {
    fileTree.innerHTML = "";
    const err = document.createElement("div");
    err.className = "sidebar-empty";
    err.textContent = String(e?.message ?? e);
    fileTree.appendChild(err);
  }
}

/** Called after each turn: updates the Files tab count, and the tree itself
 * (structure and badges, keeping open folders) if one has been loaded. */
async function refreshTreeBadgesIfVisible() {
  if (!currentWorkspace) return;
  await loadChangedPaths();
  // The count on the Files tab itself updates regardless of which sidebar
  // panel is showing — that's the whole point, noticing without switching
  // tabs. The actual tree rows only get touched if they're on screen.
  filesBadge.textContent = String(changedPaths.size);
  filesBadge.classList.toggle("hidden", changedPaths.size === 0);
  if (!fileTree.hasChildNodes()) return;
  if (sidebarPanelFiles.classList.contains("hidden")) treeStale = true;
  else await refreshFileTree();
}

async function fsListDir(path: string): Promise<{ name: string; path: string; isDir: boolean }[]> {
  const r: any = await invoke("kara_request", { method: "fs/list", params: { path } });
  return r.entries ?? [];
}

/** Folders the user has open, so a refresh rebuilds them expanded. */
const expandedDirs = new Set<string>();
/** Set when files changed while the tree wasn't on screen. */
let treeStale = false;

async function renderTreeLevel(container: HTMLElement, entries: { name: string; path: string; isDir: boolean }[], depth: number) {
  for (const entry of entries) {
    const row = document.createElement("button");
    row.className = "tree-row";
    row.style.paddingLeft = `${6 + depth * 14}px`;
    const icon = document.createElement("span");
    icon.className = "tree-icon";
    icon.textContent = treeIcon(entry.isDir, expandedDirs.has(entry.path));
    const name = document.createElement("span");
    name.className = "tree-name";
    name.textContent = entry.name;
    row.dataset.path = entry.path;
    row.dataset.depth = String(depth);
    row.appendChild(icon);
    row.appendChild(name);

    const changeKind = changedPaths.get(entry.path);
    if (changeKind) {
      const badge = document.createElement("span");
      badge.className = "tree-badge " + (changeKind === "+" ? "added" : "modified");
      badge.textContent = changeKind;
      row.appendChild(badge);
    }
    container.appendChild(row);
    row.addEventListener("contextmenu", (ev) => {
      ev.preventDefault();
      showTreeMenu(ev.clientX, ev.clientY, entry.path, entry.isDir);
    });

    if (!entry.isDir) {
      row.addEventListener("click", () => void openFileTab(entry.path));
      continue;
    }

    let childContainer: HTMLElement | undefined;
    const expand = async () => {
      childContainer = document.createElement("div");
      childContainer.className = "sidebar-empty";
      childContainer.style.paddingLeft = `${20 + depth * 14}px`;
      childContainer.textContent = "Loading…";
      row.insertAdjacentElement("afterend", childContainer);
      try {
        const children = await fsListDir(entry.path);
        childContainer.className = "";
        childContainer.removeAttribute("style");
        childContainer.textContent = "";
        await renderTreeLevel(childContainer, children, depth + 1);
      } catch (e: any) {
        childContainer.textContent = String(e?.message ?? e);
      }
    };
    row.addEventListener("click", async () => {
      const open = !expandedDirs.has(entry.path);
      if (open) expandedDirs.add(entry.path);
      else expandedDirs.delete(entry.path);
      icon.textContent = treeIcon(true, open);
      if (open) await expand();
      else {
        childContainer?.remove();
        childContainer = undefined;
      }
    });
    if (expandedDirs.has(entry.path)) await expand();
  }
}

/** Rebuilds the tree off-screen with the same folders open, then swaps it
 * in, so files Kara created or deleted show up without a flicker or losing
 * the user's place. */
async function refreshFileTree() {
  if (!currentWorkspace) return;
  treeStale = false;
  try {
    const [entries] = await Promise.all([fsListDir(""), loadChangedPaths()]);
    const next = document.createElement("div");
    await renderTreeLevel(next, entries, 0);
    const scroll = sidebar.scrollTop;
    fileTree.replaceChildren(...next.childNodes);
    sidebar.scrollTop = scroll;
  } catch {
    // Keep the current tree; the next refresh retries.
  }
}

// ── Editor tabs: Chat (permanent) + open files/diffs ─────────────

interface OpenTab {
  id: string;
  label: string;
  kind: "file" | "diff";
  path: string;
  content?: { before: string; after: string } | string;
  /** fs/read's content hash, sent back on save to detect on-disk changes. */
  version?: string;
  /** Changed on disk while it had unsaved edits; warned about once. */
  staleWarned?: boolean;
  readOnly?: boolean;
  dirty?: boolean;
}

const openTabs: OpenTab[] = [];
/** Recently closed file tabs, for Cmd/Ctrl+Shift+T. */
const closedFiles: string[] = [];
let activeTabId = "chat";

function renderEditorTabs() {
  editorTabsEl.innerHTML = "";
  const chatTab = document.createElement("button");
  chatTab.className = "editor-tab" + (activeTabId === "chat" ? " active" : "");
  chatTab.textContent = "Chat";
  chatTab.addEventListener("click", () => activateTab("chat"));
  editorTabsEl.appendChild(chatTab);

  for (const tab of openTabs) {
    const btn = document.createElement("button");
    btn.className = "editor-tab" + (activeTabId === tab.id ? " active" : "");
    const label = document.createElement("span");
    label.textContent = tab.label;
    if (tab.dirty) btn.classList.add("dirty");
    const close = document.createElement("span");
    close.className = "tab-close";
    close.textContent = "✕";
    close.addEventListener("click", (ev) => {
      ev.stopPropagation();
      closeTab(tab.id);
    });
    btn.appendChild(label);
    btn.appendChild(close);
    btn.addEventListener("click", () => activateTab(tab.id));
    editorTabsEl.appendChild(btn);
  }
}

async function activateTab(id: string, line?: number) {
  activeTabId = id;
  renderEditorTabs();
  const isChat = id === "chat";
  transcript.classList.toggle("hidden", !isChat);
  composerWrap.classList.toggle("hidden", !isChat);
  editorContainer.classList.toggle("hidden", isChat);
  cursorPos.classList.toggle("hidden", !openTabs.some((t) => t.id === id && t.kind === "file"));
  if (isChat) return;
  const tab = openTabs.find((t) => t.id === id);
  if (!tab) return;
  const { showFile, showDiff, monacoThemeFor, onCursorChange } = await loadEditorModule();
  onCursorChange((line, col) => {
    cursorPos.textContent = `Ln ${line}, Col ${col}`;
  });
  const theme = monacoThemeFor(effectiveTheme());
  if (tab.kind === "file") {
    showFile(editorContainer, tab.path, tab.content as string, theme, {
      line,
      readOnly: tab.readOnly,
      onChange: () => {
        if (tab.dirty) return;
        tab.dirty = true;
        renderEditorTabs();
      },
    });
  } else {
    const c = tab.content as { before: string; after: string };
    showDiff(editorContainer, tab.path, c.before, c.after, theme);
  }
}

async function closeTab(id: string) {
  const idx = openTabs.findIndex((t) => t.id === id);
  if (idx === -1) return;
  const tab = openTabs[idx];
  if (tab.dirty) {
    const discard = await ask(`${tab.label} has unsaved changes. Close it and discard them?`, {
      title: "Unsaved changes",
      kind: "warning",
      okLabel: "Discard",
      cancelLabel: "Keep editing",
    });
    if (!discard) return;
  }
  openTabs.splice(openTabs.indexOf(tab), 1);
  if (tab.kind === "file") {
    editorModule?.closeFileModel(tab.path);
    closedFiles.push(tab.path);
    if (closedFiles.length > 20) closedFiles.shift();
  }
  if (activeTabId === id) activateTab("chat");
  else renderEditorTabs();
}

async function saveActiveTab() {
  const tab = openTabs.find((t) => t.id === activeTabId);
  if (tab) await saveTab(tab);
}

/** Saves one file tab if it has unsaved edits; false if that failed. */
async function saveTab(tab: OpenTab): Promise<boolean> {
  if (tab.kind !== "file" || tab.readOnly || !tab.dirty || !editorModule) return true;
  const content = editorModule.fileContent(tab.path);
  if (content === undefined) return true;
  try {
    const r: any = await invoke("kara_request", {
      method: "fs/write",
      params: { path: tab.path, content, expectedVersion: tab.version },
    });
    tab.version = r.version;
    tab.content = content;
    void refreshGitPill();
    // Only clear the dot if nothing was typed while the save was in flight.
    tab.dirty = editorModule.fileContent(tab.path) !== content;
    renderEditorTabs();
    return true;
  } catch (e: any) {
    activity(`Couldn't save ${tab.path}: ${e?.message ?? e}`, "err");
    return false;
  }
}

async function saveAllTabs() {
  const dirty = openTabs.filter((t) => t.kind === "file" && t.dirty && !t.readOnly);
  let saved = 0;
  for (const t of dirty) if (await saveTab(t)) saved++;
  if (dirty.length > 1 || saved < dirty.length) {
    activity(saved === dirty.length ? `Saved ${saved} files.` : `Saved ${saved} of ${dirty.length} files; the others are still unsaved.`, saved === dirty.length ? "dim" : "warn");
  }
}

async function openFileTab(path: string, line?: number) {
  const id = `file:${path}`;
  if (openTabs.some((t) => t.id === id)) {
    activateTab(id, line);
    return;
  }
  try {
    const r: any = await invoke("kara_request", { method: "fs/read", params: { path } });
    if (r.binary) {
      activity(`${path} looks binary — nothing to show as text.`, "warn");
      return;
    }
    const label = path.split("/").pop() ?? path;
    openTabs.push({
      id,
      label: r.truncated ? `${label} (read-only, truncated)` : r.utf8 === false ? `${label} (read-only, not UTF-8)` : label,
      kind: "file",
      path,
      content: r.content ?? "",
      version: r.version,
      // Saving would cut a truncated file short, or corrupt one in another
      // encoding with the replacement characters it was shown with.
      readOnly: !!r.truncated || r.utf8 === false,
    });
    activateTab(id, line);
  } catch (e: any) {
    activity(`error: ${e?.message ?? e}`, "err");
  }
}

async function openDiffTab(path: string) {
  const id = `diff:${path}`;
  try {
    const r: any = await invoke("kara_request", { method: "session/changes", params: {} });
    const match = (r.kara ?? []).find((f: any) => f.path === path);
    if (!match) {
      activity(`No diff available for ${path} anymore.`, "warn");
      return;
    }
    const existing = openTabs.find((t) => t.id === id);
    const content = { before: match.before ?? "", after: match.after ?? "" };
    if (existing) {
      existing.content = content;
    } else {
      const label = path.split("/").pop() ?? path;
      openTabs.push({ id, label: `${label} (diff)`, kind: "diff", path, content });
    }
    activateTab(id);
  } catch (e: any) {
    if (String(e?.message ?? e) === "busy") {
      pendingDiffPath = path;
      activity(`Kara is still working; ${path.split("/").pop()}'s diff opens when this turn finishes.`, "dim");
      return;
    }
    activity(`error: ${e?.message ?? e}`, "err");
  }
}

async function boot() {
  markPlatform();
  await listen("kara://event", (e) => handleEvent(e.payload));
  await listen("kara://log", (e: any) => {
    if (e.payload?.level === "progress") {
      updateDownloadProgress(e.payload);
      return;
    }
    activity(`[${e.payload?.level ?? "info"}] ${e.payload?.message ?? ""}`, "dim");
  });
  await listen("kara://protocol-error", (e) => activity(`protocol error: ${e.payload}`, "err"));
  await listen("kara://stderr", () => {
    /* surfaced in the Rust side's own logs; not shown in-window for v1 */
  });
  await listen("kara://exited", () => {
    sessionState.textContent = "Kara exited";
    modelPill.textContent = "stopped";
    modelPill.className = "pill";
    setProjectControlsEnabled(false);
    activity("Kara's process exited unexpectedly.", "err");
    showRestartButton();
  });
  await listen("kara://request", async (e: any) => {
    const { id, method, params } = e.payload;
    if (method === "permission/request") {
      const allow = window.confirm(
        `Kara wants to: ${params?.title ?? "do something"}\n\n${(params?.reasons ?? []).join("\n")}\n\nAllow once?`,
      );
      await invoke("kara_respond", { id, decision: allow ? { decision: "allow_once" } : { decision: "deny" } });
    } else {
      await invoke("kara_respond", { id, error: `unsupported request ${method}` });
    }
  });

  let last = "";
  try {
    last = localStorage.getItem(LAST_WORKSPACE_KEY) ?? "";
  } catch {
    /* ignore */
  }
  setProjectControlsEnabled(false);
  if (last) {
    await startWorkspace(last);
  } else {
    // No remembered workspace: wait for the user to actually pick one,
    // rather than defaulting to the home directory. Confirmed by running
    // it that "default to $HOME" means indexing the user's entire home
    // folder with full parallelism the moment the window opens — not a
    // reasonable default on any machine, fast or not.
    sessionState.textContent = "no project open";
    emptyText.textContent = "Open a project to get started.";
    emptyOpenBtn.classList.remove("hidden");
    refreshRecentSidebar();
  }
}

boot();

// ── Quick open (Cmd+P) and command palette (Shift+Cmd+P) ─────────

interface PaletteItem {
  label: string;
  detail?: string;
  run: () => void;
}

const paletteBackdrop = document.querySelector<HTMLElement>("#palette-backdrop")!;
const paletteInput = document.querySelector<HTMLInputElement>("#palette-input")!;
const paletteList = document.querySelector<HTMLUListElement>("#palette-list")!;

let paletteSelected = 0;
let paletteRows: PaletteItem[] = [];

function paletteCommands(): PaletteItem[] {
  const mode = (m: string) => () => document.querySelector<HTMLButtonElement>(`.mode-opt[data-mode="${m}"]`)?.click();
  const cmds: PaletteItem[] = [
    { label: "Toggle terminal", detail: "⌘J", run: () => void toggleTerminal() },
    { label: "Find in files", detail: "⇧⌘F", run: () => showSidebarPanel("search") },
    { label: "Show file explorer", run: () => showSidebarPanel("files") },
    { label: "Toggle sidebar", detail: "⌘B", run: toggleSidebar },
    { label: "Toggle light/dark theme", run: toggleTheme },
    { label: "Save file", detail: "⌘S", run: () => void saveActiveTab() },
    { label: "Save all", detail: "⌥⌘S", run: () => void saveAllTabs() },
    { label: "New session", detail: "⌘N", run: () => void newSession() },
    { label: "Open project…", detail: "⌘O", run: () => void pickProject() },
    { label: "Settings", detail: "⌘,", run: () => void openSettings() },
    { label: "Undo Kara's last change", run: () => void undo() },
    { label: "Reopen closed tab", detail: "⇧⌘T", run: () => { const p = closedFiles.pop(); if (p) void openFileTab(p); } },
    ...activeFileCommands(),
    { label: "Mode: Execute", detail: "understand, edit, test, review", run: mode("execute") },
    { label: "Mode: Plan", detail: "read-only, propose a plan", run: mode("plan") },
    { label: "Mode: Review", detail: "read-only risk review", run: mode("review") },
  ];
  for (const c of availableCommands) {
    cmds.push({
      label: c.name,
      detail: c.description,
      run: () => {
        activateTab("chat");
        input.value = c.name + " ";
        autosize();
        input.focus();
      },
    });
  }
  return cmds;
}

/** Commands about the file in the active tab, when there is one. */
function activeFileCommands(): PaletteItem[] {
  const tab = openTabs.find((t) => t.id === activeTabId);
  if (!tab) return [];
  const reveal = navigator.platform.toLowerCase().includes("mac") ? "Finder" : currentWorkspace.includes("\\") ? "File Explorer" : "file manager";
  return [
    { label: "Copy path of active file", detail: tab.path, run: () => void navigator.clipboard.writeText(absolutePath(tab.path)) },
    { label: "Copy relative path of active file", detail: tab.path, run: () => void navigator.clipboard.writeText(tab.path) },
    { label: `Reveal active file in ${reveal}`, detail: tab.path, run: () => void revealItemInDir(absolutePath(tab.path)).catch((e) => activity(String(e), "err")) },
  ];
}

async function openPalette(kind: "files" | "commands") {
  if (kind === "files" && !currentWorkspace) kind = "commands";
  paletteBackdrop.classList.remove("hidden");
  paletteInput.value = kind === "commands" ? ">" : "";
  paletteInput.placeholder = "Search files by name (type > for commands)";
  paletteInput.focus();
  renderPalette();
  if (kind === "files" && !projectFiles) {
    try {
      const r: any = await invoke("kara_request", { method: "fs/files", params: {} });
      projectFiles = r.files ?? [];
    } catch (e: any) {
      projectFiles = [];
      activity(`Couldn't list project files: ${e?.message ?? e}`, "err");
    }
    renderPalette();
  }
}

function closePalette() {
  paletteBackdrop.classList.add("hidden");
}

function highlighted(text: string, positions: number[]): DocumentFragment {
  const frag = document.createDocumentFragment();
  const hit = new Set(positions);
  let run = "";
  let runHit = false;
  const flush = () => {
    if (!run) return;
    if (runHit) {
      const b = document.createElement("b");
      b.textContent = run;
      frag.appendChild(b);
    } else frag.appendChild(document.createTextNode(run));
    run = "";
  };
  for (let i = 0; i < text.length; i++) {
    if (hit.has(i) !== runHit) {
      flush();
      runHit = hit.has(i);
    }
    run += text[i];
  }
  flush();
  return frag;
}

function renderPalette() {
  const raw = paletteInput.value;
  const commandMode = raw.startsWith(">");
  const query = commandMode ? raw.slice(1).trim() : raw.trim();
  paletteList.replaceChildren();
  paletteRows = [];

  if (commandMode) {
    for (const { item, match } of fuzzyFilter(query, paletteCommands(), (c) => c.label, 50)) {
      paletteRows.push(item);
      appendPaletteRow(highlighted(item.label, match.positions), item.detail);
    }
  } else if (!projectFiles) {
    appendPaletteMessage("Loading files…");
  } else {
    for (const { item, match } of fuzzyFilter(query, projectFiles, (f) => f, 50)) {
      paletteRows.push({ label: item, run: () => void openFileTab(item) });
      const slash = item.lastIndexOf("/");
      const name = highlighted(item.slice(slash + 1), match.positions.filter((i) => i > slash).map((i) => i - slash - 1));
      appendPaletteRow(name, slash > 0 ? item.slice(0, slash) : undefined);
    }
  }
  if (!paletteRows.length && (commandMode || projectFiles)) appendPaletteMessage(commandMode ? "No matching commands" : "No matching files");
  paletteSelected = 0;
  updatePaletteSelection();
}

function appendPaletteRow(label: DocumentFragment, detail?: string) {
  const index = paletteRows.length - 1;
  const li = document.createElement("li");
  li.className = "palette-row";
  li.setAttribute("role", "option");
  const l = document.createElement("span");
  l.className = "palette-label";
  l.appendChild(label);
  li.appendChild(l);
  if (detail) {
    const d = document.createElement("span");
    d.className = "palette-detail";
    d.textContent = detail;
    li.appendChild(d);
  }
  li.addEventListener("mousemove", () => {
    if (paletteSelected === index) return;
    paletteSelected = index;
    updatePaletteSelection();
  });
  li.addEventListener("click", () => runPaletteRow(index));
  paletteList.appendChild(li);
}

function appendPaletteMessage(text: string) {
  const li = document.createElement("li");
  li.className = "palette-empty";
  li.textContent = text;
  paletteList.appendChild(li);
}

function updatePaletteSelection() {
  paletteList.querySelectorAll<HTMLElement>(".palette-row").forEach((el, i) => {
    const on = i === paletteSelected;
    el.classList.toggle("selected", on);
    el.setAttribute("aria-selected", String(on));
    if (on) el.scrollIntoView({ block: "nearest" });
  });
}

function runPaletteRow(index: number) {
  const item = paletteRows[index];
  if (!item) return;
  closePalette();
  item.run();
}

paletteInput.addEventListener("input", renderPalette);
paletteInput.addEventListener("keydown", (e) => {
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    if (!paletteRows.length) return;
    const step = e.key === "ArrowDown" ? 1 : -1;
    paletteSelected = (paletteSelected + step + paletteRows.length) % paletteRows.length;
    updatePaletteSelection();
  } else if (e.key === "Enter") {
    e.preventDefault();
    runPaletteRow(paletteSelected);
  } else if (e.key === "Escape") {
    e.preventDefault();
    closePalette();
  }
});
paletteBackdrop.addEventListener("mousedown", (e) => {
  if (e.target === paletteBackdrop) closePalette();
});

// ── Settings: where inference runs ───────────────────────────────

const SERVER_KINDS: [string, string, string][] = [
  ["ollama", "Ollama", "http://127.0.0.1:11434/v1"],
  ["lmstudio", "LM Studio", "http://127.0.0.1:1234/v1"],
  ["vllm", "vLLM", "http://127.0.0.1:8000/v1"],
  ["openai_compat", "Other OpenAI-compatible server", "https://…/v1"],
];

function field(labelText: string, el: HTMLInputElement | HTMLSelectElement): HTMLElement {
  const wrap = document.createElement("label");
  wrap.className = "settings-field";
  const l = document.createElement("span");
  l.textContent = labelText;
  wrap.append(l, el);
  return wrap;
}

function textInput(placeholder: string, type = "text"): HTMLInputElement {
  const i = document.createElement("input");
  i.type = type;
  i.placeholder = placeholder;
  i.spellcheck = false;
  i.autocomplete = "off";
  return i;
}

function renderInferenceSettings() {
  const label = document.createElement("div");
  label.className = "panel-section-label";
  label.textContent = "Where inference runs";
  panelBody.appendChild(label);

  const status = document.createElement("div");
  status.className = "settings-status";
  const form = document.createElement("div");
  form.className = "settings-form";

  const isServer = SERVER_KINDS.some(([k]) => k === inferenceProvider);
  const choices: [string, string, string, boolean][] = [
    ["local", "This machine", "Local models managed by Kara (see the model picker).", inferenceProvider === "local"],
    ["kara", "Another Kara machine", "A machine of yours running kara serve --inference.", inferenceProvider === "kara"],
    ["server", "A model server", "Ollama, LM Studio, vLLM or any OpenAI-compatible server.", isServer],
  ];
  const rows: HTMLElement[] = [];
  for (const [kind, title, meta, selected] of choices) {
    const row = document.createElement("div");
    row.className = "perm-option" + (selected ? " selected" : "");
    const left = document.createElement("div");
    const t = document.createElement("div");
    t.className = "title";
    t.textContent = title;
    const m = document.createElement("div");
    m.className = "meta";
    m.textContent = meta;
    left.append(t, m);
    row.appendChild(left);
    row.addEventListener("click", () => {
      rows.forEach((r) => r.classList.toggle("selected", r === row));
      showForm(kind);
    });
    rows.push(row);
    panelBody.appendChild(row);
  }
  panelBody.append(form, status);

  const apply = async (button: HTMLButtonElement, steps: () => Promise<string>) => {
    button.disabled = true;
    status.className = "settings-status";
    status.textContent = "Working…";
    try {
      const msg = await steps();
      status.textContent = msg.split("\n")[0] || "Saved.";
      activity(status.textContent, "dim");
      if (currentWorkspace) await startWorkspace(currentWorkspace);
    } catch (e: any) {
      status.className = "settings-status err";
      status.textContent = String(e?.message ?? e);
    } finally {
      button.disabled = false;
    }
  };
  const set = (key: string, value: string) => invoke<string>("kara_config_set", { key, value });

  function showForm(kind: string) {
    form.replaceChildren();
    status.textContent = "";
    const button = document.createElement("button");
    button.className = "approve-btn";
    if (kind === "local") {
      button.textContent = "Use this machine";
      button.addEventListener("click", () => void apply(button, () => set("inference.provider", "local")));
      form.appendChild(button);
    } else if (kind === "kara") {
      const url = textInput("http://gpu-box:7878");
      const token = textInput("printed by kara serve --inference", "password");
      button.textContent = "Connect";
      button.addEventListener("click", () => void apply(button, () => invoke<string>("kara_connect", { url: url.value, token: token.value })));
      form.append(field("Address", url), field("Token", token), button);
    } else {
      const select = document.createElement("select");
      for (const [k, name] of SERVER_KINDS) {
        const o = document.createElement("option");
        o.value = k;
        o.textContent = name;
        select.appendChild(o);
      }
      if (isServer) select.value = inferenceProvider;
      const url = textInput("");
      const model = textInput("as the server names it, e.g. qwen3:8b (empty: its first model)");
      const syncPlaceholder = () => {
        const kindInfo = SERVER_KINDS.find(([k]) => k === select.value)!;
        url.placeholder = select.value === "openai_compat" ? kindInfo[2] : `${kindInfo[2]} (default)`;
      };
      select.addEventListener("change", syncPlaceholder);
      syncPlaceholder();
      button.textContent = "Save";
      button.addEventListener("click", () =>
        void apply(button, async () => {
          if (select.value === "openai_compat" && !url.value.trim()) throw new Error("This server needs its address.");
          await set("inference.provider", select.value);
          await set("inference.endpoint", url.value.trim());
          await set("inference.model", model.value.trim());
          return `Inference now uses ${select.selectedOptions[0].textContent}.`;
        }),
      );
      form.append(field("Server", select), field("Address", url), field("Model", model), button);
    }
  }
}

// ── File tree context menu ───────────────────────────────────────

function absolutePath(rel: string): string {
  return joinWorkspacePath(currentWorkspace, rel);
}

let treeMenu: HTMLElement | undefined;

function hideTreeMenu() {
  treeMenu?.remove();
  treeMenu = undefined;
}

function showTreeMenu(x: number, y: number, rel: string, isDir: boolean) {
  hideTreeMenu();
  const menu = document.createElement("div");
  menu.className = "context-menu";
  menu.setAttribute("role", "menu");
  const reveal = navigator.platform.toLowerCase().includes("mac") ? "Reveal in Finder" : currentWorkspace.includes("\\") ? "Reveal in File Explorer" : "Reveal in file manager";
  const folder = isDir ? rel : rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/")) : "";
  const items: [string, () => void | Promise<void>][] = [
    ...(isDir ? [] : ([["Open", () => openFileTab(rel)]] as [string, () => Promise<void>][])),
    ["New file…", () => startTreeCreate(folder, "file")],
    ["New folder…", () => startTreeCreate(folder, "dir")],
    ["Rename…", () => startTreeRename(rel)],
    [currentWorkspace.includes("\\") ? "Move to Recycle Bin" : "Move to Trash", () => trashTreeItem(rel, isDir)],
    ["Copy path", () => navigator.clipboard.writeText(absolutePath(rel))],
    ["Copy relative path", () => navigator.clipboard.writeText(rel)],
    [reveal, () => revealItemInDir(absolutePath(rel))],
  ];
  for (const [label, run] of items) {
    const item = document.createElement("button");
    item.className = "context-menu-item";
    item.setAttribute("role", "menuitem");
    item.textContent = label;
    item.addEventListener("click", async () => {
      hideTreeMenu();
      try {
        await run();
      } catch (e: any) {
        activity(`${label} failed: ${e?.message ?? e}`, "err");
      }
    });
    menu.appendChild(item);
  }
  document.body.appendChild(menu);
  // Keep it on screen near the window edges.
  const r = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(x, window.innerWidth - r.width - 4)}px`;
  menu.style.top = `${Math.min(y, window.innerHeight - r.height - 4)}px`;
  treeMenu = menu;
}

window.addEventListener("mousedown", (e) => {
  if (treeMenu && !treeMenu.contains(e.target as Node)) hideTreeMenu();
});
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") hideTreeMenu();
});
window.addEventListener("blur", hideTreeMenu);

// ── Drop a folder on the window to open it ───────────────────────

void getCurrentWebview().onDragDropEvent(async (event) => {
  const p = event.payload;
  if (p.type === "over") {
    document.body.classList.add("drop-target");
    return;
  }
  document.body.classList.remove("drop-target");
  if (p.type !== "drop" || p.paths.length === 0) return;
  const dir = p.paths[0];
  if (busy) {
    activity("Kara is working; stop the current turn before switching projects.", "warn");
    return;
  }
  if (!(await invoke<boolean>("path_is_dir", { path: dir }).catch(() => false))) {
    activity("Drop a folder to open it as a project.", "dim");
    return;
  }
  if (dir !== currentWorkspace) await openWorkspace(dir);
});

// Quitting must not silently drop unsaved edits either.
void getCurrentWindow().onCloseRequested(async (event) => {
  if (!(await confirmDiscardUnsaved("Quit"))) event.preventDefault();
});

// ── File tree: new file / folder, rename ─────────────────────────

/** An inline name box in the tree, like VS Code's: Enter commits, Esc or
 * clicking away cancels. */
function inlineNameInput(after: Element | null, parent: HTMLElement, depth: number, initial: string, commit: (name: string) => Promise<void>) {
  const box = document.createElement("input");
  box.className = "tree-input";
  box.value = initial;
  box.spellcheck = false;
  box.style.marginLeft = `${6 + depth * 14}px`;
  if (after) after.insertAdjacentElement("afterend", box);
  else parent.prepend(box);
  box.focus();
  const dot = initial.lastIndexOf(".");
  box.setSelectionRange(0, dot > 0 ? dot : initial.length);
  let done = false;
  const finish = async (ok: boolean) => {
    if (done) return;
    done = true;
    const name = box.value.trim();
    box.remove();
    if (ok && name && name !== initial) {
      try {
        await commit(name);
      } catch (e: any) {
        activity(String(e?.message ?? e), "err");
      }
    }
  };
  box.addEventListener("keydown", (e) => {
    if (e.key === "Enter") void finish(true);
    else if (e.key === "Escape") void finish(false);
    e.stopPropagation();
  });
  box.addEventListener("blur", () => void finish(false));
}

const treeRow = (rel: string) => [...fileTree.querySelectorAll<HTMLElement>(".tree-row")].find((r) => r.dataset.path === rel);

async function startTreeCreate(folder: string, kind: "file" | "dir") {
  if (folder && !expandedDirs.has(folder)) {
    treeRow(folder)?.click();
    await new Promise((r) => setTimeout(r, 150));
  }
  const anchor = folder ? treeRow(folder) ?? null : null;
  const depth = anchor ? Number(anchor.dataset.depth ?? 0) + 1 : 0;
  inlineNameInput(anchor, fileTree, depth, "", async (name) => {
    const path = folder ? `${folder}/${name}` : name;
    await invoke("kara_request", { method: "fs/create", params: { path, kind } });
    await refreshFileTree();
    if (kind === "file") await openFileTab(path);
  });
}

function startTreeRename(rel: string) {
  const row = treeRow(rel);
  if (!row) return;
  const name = rel.split("/").pop() ?? rel;
  const parent = rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/")) : "";
  const affected = openTabs.filter((t) => t.kind === "file" && (t.path === rel || t.path.startsWith(rel + "/")));
  if (affected.some((t) => t.dirty)) {
    activity(`Save or close ${affected.find((t) => t.dirty)!.label} before renaming ${name}.`, "warn");
    return;
  }
  row.style.display = "none";
  inlineNameInput(row, fileTree, Number(row.dataset.depth ?? 0), name, async (newName) => {
    const to = parent ? `${parent}/${newName}` : newName;
    try {
      await invoke("kara_request", { method: "fs/rename", params: { from: rel, to } });
    } finally {
      row.style.display = "";
    }
    // Reopen moved tabs under their new paths.
    const reopen = affected.map((t) => to + t.path.slice(rel.length));
    for (const t of affected) await closeTab(t.id);
    if (expandedDirs.delete(rel)) expandedDirs.add(to);
    await refreshFileTree();
    for (const p of reopen) await openFileTab(p);
  });
  // A cancelled rename must bring the row back.
  const box = row.nextElementSibling as HTMLInputElement | null;
  box?.addEventListener("blur", () => (row.style.display = ""));
  box?.addEventListener("keydown", (e) => {
    if (e.key === "Escape") row.style.display = "";
  });
}

async function trashTreeItem(rel: string, isDir: boolean) {
  const name = rel.split("/").pop() ?? rel;
  const affected = openTabs.filter((t) => t.kind === "file" && (t.path === rel || t.path.startsWith(rel + "/")));
  const unsaved = affected.filter((t) => t.dirty).length;
  const bin = currentWorkspace.includes("\\") ? "Recycle Bin" : "Trash";
  const ok = await ask(
    `Move ${isDir ? "the folder " : ""}${name} to the ${bin}?` + (unsaved ? ` ${unsaved} open file${unsaved === 1 ? " has" : "s have"} unsaved changes that will be lost.` : ""),
    { title: `Move to ${bin}`, kind: "warning", okLabel: `Move to ${bin}`, cancelLabel: "Cancel" },
  );
  if (!ok) return;
  try {
    await invoke("kara_request", { method: "fs/trash", params: { path: rel } });
  } catch (e: any) {
    activity(String(e?.message ?? e), "err");
    return;
  }
  for (const t of affected) {
    t.dirty = false; // already confirmed above; don't ask again on close
    await closeTab(t.id);
  }
  expandedDirs.delete(rel);
  await refreshFileTree();
}

// ── Open files follow changes on disk ────────────────────────────

/** Re-reads open file tabs (all of them, or those in `paths`) and updates
 * the ones whose content changed on disk: a tab without unsaved edits is
 * reloaded in place; one with edits keeps them and gets a warning, and its
 * save will be refused as before. */
async function reloadOpenFiles(paths?: string[]) {
  if (!editorModule) return;
  const wanted = paths ? new Set(paths) : undefined;
  for (const tab of openTabs) {
    if (tab.kind !== "file" || tab.readOnly || (wanted && !wanted.has(tab.path))) continue;
    let r: any;
    try {
      r = await invoke("kara_request", { method: "fs/read", params: { path: tab.path } });
    } catch {
      continue; // deleted or unreadable; the tab stays as it was
    }
    if (!r.version || r.version === tab.version || r.binary) continue;
    if (tab.dirty) {
      if (!tab.staleWarned) {
        tab.staleWarned = true;
        activity(`${tab.label} changed on disk while you were editing it. Your edits are kept; reopen the file to see the new version.`, "warn");
      }
      continue;
    }
    editorModule.replaceFileContent(tab.path, r.content ?? "");
    tab.content = r.content ?? "";
    tab.version = r.version;
    // replaceFileContent fires the change listener; this wasn't the user.
    tab.dirty = false;
  }
  renderEditorTabs();
}

// Edits made in another editor while Kara was in the background.
window.addEventListener("focus", () => {
  void reloadOpenFiles();
  void refreshGitPill();
});

// ── Git branch in the status bar ─────────────────────────────────

async function refreshGitPill() {
  if (!currentWorkspace) {
    gitPill.classList.add("hidden");
    return;
  }
  try {
    const g: any = await invoke("kara_request", { method: "git/info", params: {} });
    if (!g.repo) {
      gitPill.classList.add("hidden");
      return;
    }
    const where = g.branch ?? (g.head ? `detached at ${g.head}` : "no commits yet");
    gitPill.textContent = `⎇ ${where}` + (g.changed ? ` · ${g.changed} changed` : "");
    gitPill.title = g.branch ? `Branch ${g.branch}${g.head ? ` at ${g.head}` : ""}` : where;
    gitPill.classList.remove("hidden");
  } catch {
    // An older kara without git/info, or no session yet: just hide it.
    gitPill.classList.add("hidden");
  }
}

/** Diff tabs show Kara's change as of when they were opened; after a turn
 * the change may have grown, been reverted or rejected. Re-read them. */
async function refreshDiffTabs() {
  const diffs = openTabs.filter((t) => t.kind === "diff");
  if (!diffs.length) return;
  let changes: any[];
  try {
    const r: any = await invoke("kara_request", { method: "session/changes", params: {} });
    changes = r.kara ?? [];
  } catch {
    return;
  }
  for (const tab of diffs) {
    const match = changes.find((f: any) => f.path === tab.path);
    if (!match) {
      tab.label = `${tab.path.split("/").pop()} (diff, no longer changed)`;
      continue;
    }
    tab.content = { before: match.before ?? "", after: match.after ?? "" };
    if (tab.id === activeTabId) void activateTab(tab.id);
  }
  renderEditorTabs();
}

