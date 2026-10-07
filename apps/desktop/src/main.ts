// Kara desktop: a thin window over `kara serve --stdio`. All agent logic
// lives in the Rust core; this file only renders events and sends prompts,
// mirroring the VS Code extension's chatView.ts for the same protocol.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

const transcript = document.querySelector<HTMLElement>("#transcript")!;
const emptyState = document.querySelector<HTMLElement>("#empty-state")!;
const input = document.querySelector<HTMLTextAreaElement>("#input")!;
const sendBtn = document.querySelector<HTMLButtonElement>("#send")!;
const sendLabel = document.querySelector<HTMLElement>("#send-label")!;
const sessionState = document.querySelector<HTMLElement>("#session-state")!;
const modelPill = document.querySelector<HTMLButtonElement>("#model-pill")!;
const permPill = document.querySelector<HTMLButtonElement>("#perm-pill")!;
const contextPill = document.querySelector<HTMLButtonElement>("#context-pill")!;
const btnNew = document.querySelector<HTMLButtonElement>("#btn-new")!;
const btnUndo = document.querySelector<HTMLButtonElement>("#btn-undo")!;
const btnProject = document.querySelector<HTMLButtonElement>("#btn-project")!;
const btnSessions = document.querySelector<HTMLButtonElement>("#btn-sessions")!;
const btnSettings = document.querySelector<HTMLButtonElement>("#btn-settings")!;
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
let cmdMenuIndex = 0;
let permissionsMode = "";
let turnMode: "execute" | "plan" | "review" = "execute";
const modeOpts = document.querySelectorAll<HTMLButtonElement>(".mode-opt");
const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const LAST_WORKSPACE_KEY = "kara.lastWorkspace";
const THEME_KEY = "kara.theme";

type Theme = "system" | "light" | "dark";

function currentTheme(): Theme {
  try {
    const v = localStorage.getItem(THEME_KEY);
    if (v === "light" || v === "dark") return v;
  } catch {
    /* ignore */
  }
  return "system";
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

// ── Transcript rendering ──────────────────────────────────────────

function clearEmptyState() {
  emptyState.style.display = "none";
}

function scrollToBottom() {
  transcript.scrollTop = transcript.scrollHeight;
}

function bubble(text: string, who: "user" | "assistant"): HTMLElement {
  clearEmptyState();
  const row = document.createElement("div");
  row.className = `bubble-row from-${who}`;
  const b = document.createElement("div");
  b.className = `bubble ${who}`;
  b.textContent = text;
  row.appendChild(b);
  transcript.appendChild(row);
  scrollToBottom();
  return b;
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
  spinnerRow.className = "spinner-row";
  transcript.appendChild(spinnerRow);
  spinnerFrame = 0;
  const started = Date.now();
  spinnerTimer = window.setInterval(() => {
    spinnerFrame++;
    const secs = Math.round((Date.now() - started) / 1000);
    if (spinnerRow) {
      spinnerRow.textContent = `${SPINNER[spinnerFrame % SPINNER.length]} Kara is working… ${secs}s`;
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
  sendLabel.textContent = b ? "Stop" : "Send";
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
      pendingAssistant.textContent += e.text;
      scrollToBottom();
      break;
    case "assistant_message":
      pendingAssistant = undefined;
      break;
    case "tool_started":
      activity(`→ ${e.summary}`, "accent");
      break;
    case "tool_finished": {
      const mark = e.ok ? "✓" : "✗";
      const time = e.duration_ms >= 1000 ? ` (${(e.duration_ms / 1000).toFixed(1)}s)` : "";
      activity(`  ${mark} ${e.summary}${time}`, e.ok ? "dim" : "err");
      break;
    }
    case "file_changed":
      fileChangedRow(e.change);
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
    activity(`error: ${e?.message ?? e}`, "err");
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
        closePanel();
        modelPill.textContent = `loading ${m.name}…`;
        try {
          const sel: any = await invoke("kara_request", { method: "models/select", params: { id: m.id, download: true } });
          modelPill.textContent = sel?.available ? sel.label : "no model";
          modelPill.className = sel?.available ? "pill ready" : "pill warn";
        } catch (e: any) {
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

async function openSessions() {
  openPanel("Session history");
  try {
    const r: any = await invoke("kara_request", { method: "session/list", params: {} });
    panelBody.innerHTML = "";
    if (!r.sessions?.length) {
      const empty = document.createElement("div");
      empty.className = "panel-empty";
      empty.textContent = "No past sessions for this project.";
      panelBody.appendChild(empty);
      return;
    }
    for (const s of r.sessions) {
      const row = document.createElement("div");
      row.className = "panel-row" + (s.id === r.current ? " current" : "");
      const left = document.createElement("div");
      const title = document.createElement("div");
      title.className = "title";
      title.textContent = s.title || "(empty)";
      const meta = document.createElement("div");
      meta.className = "meta";
      meta.textContent = `${(s.updated ?? "").slice(0, 16).replace("T", " ")} · ${s.turns} turn${s.turns === 1 ? "" : "s"} · ${s.model}`;
      left.appendChild(title);
      left.appendChild(meta);
      row.appendChild(left);
      if (s.id !== r.current) {
        row.addEventListener("click", async () => {
          closePanel();
          try {
            await invoke("kara_request", { method: "session/resume", params: { id: s.id } });
            clearTranscript();
            activity(`Resumed: ${s.title || s.id.slice(0, 8)}`, "dim");
            void updateContextPill();
          } catch (e: any) {
            activity(`error: ${e?.message ?? e}`, "err");
          }
        });
      }
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

async function openSettings() {
  openPanel("Settings");

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
        permPill.textContent = mode;
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
btnSessions.addEventListener("click", openSessions);

// ── Project (folder) picker ─────────────────────────────────────────

function shortPath(p: string): string {
  const parts = p.split(/[/\\]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : p;
}

let currentWorkspace = "";

async function startWorkspace(workspace: string) {
  sessionState.textContent = "starting…";
  removeRestartButton();
  try {
    const info: any = await invoke("kara_start", { workspace });
    currentWorkspace = info?.workspace ?? workspace;
    modelPill.textContent = info?.model?.available ? info.model.label : "no model";
    modelPill.className = info?.model?.available ? "pill ready" : "pill warn";
    permissionsMode = info?.permissionsMode ?? "";
    permPill.textContent = permissionsMode || "—";
    availableCommands = info?.commands ?? [];
    btnProject.textContent = info?.workspace ? shortPath(info.workspace) : "~";
    btnProject.title = info?.workspace ?? "Open a different folder";
    sessionState.textContent = "ready";
    void updateContextPill();
    try {
      localStorage.setItem(LAST_WORKSPACE_KEY, currentWorkspace);
    } catch {
      /* private window or storage disabled: just skip remembering it */
    }
  } catch (e: any) {
    activity(`Kara failed to start: ${e?.message ?? e}`, "err");
    sessionState.textContent = "not running";
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

async function pickProject() {
  const dir = await openDialog({ directory: true, multiple: false, title: "Open a project for Kara" });
  if (!dir || typeof dir !== "string") return;
  clearTranscript();
  pendingPlan = false;
  await startWorkspace(dir);
}

btnProject.addEventListener("click", pickProject);

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

// Native-feeling shortcuts: Cmd on macOS, Ctrl elsewhere.
window.addEventListener("keydown", (e) => {
  const mod = e.metaKey || e.ctrlKey;
  if (!mod) return;
  switch (e.key.toLowerCase()) {
    case "n":
      e.preventDefault();
      newSession();
      break;
    case "k":
      e.preventDefault();
      openSessions();
      break;
    case ",":
      e.preventDefault();
      openSettings();
      break;
    case "o":
      e.preventDefault();
      pickProject();
      break;
  }
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

async function boot() {
  markPlatform();
  await listen("kara://event", (e) => handleEvent(e.payload));
  await listen("kara://log", (e: any) => activity(`[${e.payload?.level ?? "info"}] ${e.payload?.message ?? ""}`, "dim"));
  await listen("kara://protocol-error", (e) => activity(`protocol error: ${e.payload}`, "err"));
  await listen("kara://stderr", () => {
    /* surfaced in the Rust side's own logs; not shown in-window for v1 */
  });
  await listen("kara://exited", () => {
    sessionState.textContent = "Kara exited";
    modelPill.textContent = "stopped";
    modelPill.className = "pill";
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

  // v1 has no "recent projects" list — just the one remembered workspace,
  // falling back to the Rust side's home-directory default on first run.
  let last = "";
  try {
    last = localStorage.getItem(LAST_WORKSPACE_KEY) ?? "";
  } catch {
    /* ignore */
  }
  await startWorkspace(last);
}

boot();
