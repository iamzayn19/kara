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
const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const LAST_WORKSPACE_KEY = "kara.lastWorkspace";

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

function appendDiff(diff: string) {
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
  transcript.appendChild(pre);
  scrollToBottom();
}

function clearTranscript() {
  transcript.querySelectorAll(".bubble-row, .activity, .diff-block").forEach((el) => el.remove());
  emptyState.style.display = "flex";
}

let pendingAssistant: HTMLElement | undefined;

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
    case "file_changed": {
      const [add, del] = diffStat(e.change.diff);
      const sym = e.change.kind === "created" ? "+" : e.change.kind === "deleted" ? "-" : "~";
      activity(`  ${sym} ${e.change.path}  (+${add} −${del})`, "dim");
      if (add + del <= 24 && e.change.kind !== "deleted") appendDiff(e.change.diff);
      break;
    }
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
        activity("Plan ready. Reply (or \"approve\") to carry it out.", "accent");
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
      params: { text, mode: "execute", approvePlan: pendingPlan && /^(y|yes|approve|go|ok|proceed)$/i.test(text) },
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
}

modelPill.addEventListener("click", openModelPicker);
permPill.addEventListener("click", openSettings);
btnSettings.addEventListener("click", openSettings);
btnSessions.addEventListener("click", openSessions);

// ── Project (folder) picker ─────────────────────────────────────────

function shortPath(p: string): string {
  const parts = p.split(/[/\\]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : p;
}

async function startWorkspace(workspace: string) {
  sessionState.textContent = "starting…";
  try {
    const info: any = await invoke("kara_start", { workspace });
    modelPill.textContent = info?.model?.available ? info.model.label : "no model";
    modelPill.className = info?.model?.available ? "pill ready" : "pill warn";
    permissionsMode = info?.permissionsMode ?? "";
    permPill.textContent = permissionsMode || "—";
    availableCommands = info?.commands ?? [];
    btnProject.textContent = info?.workspace ? shortPath(info.workspace) : "~";
    btnProject.title = info?.workspace ?? "Open a different folder";
    sessionState.textContent = "ready";
    try {
      localStorage.setItem(LAST_WORKSPACE_KEY, info?.workspace ?? workspace);
    } catch {
      /* private window or storage disabled: just skip remembering it */
    }
  } catch (e: any) {
    activity(`Kara failed to start: ${e?.message ?? e}`, "err");
    sessionState.textContent = "not running";
  }
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
btnNew.addEventListener("click", newSession);
btnUndo.addEventListener("click", undo);

// ── Boot ──────────────────────────────────────────────────────────

/** macOS gets an overlay titlebar (native traffic lights); reserve space
 * for them in the toolbar. No navigator.platform parsing library needed
 * for a yes/no check this simple. */
function markPlatform() {
  if (/Mac/.test(navigator.platform) || /Macintosh/.test(navigator.userAgent)) {
    document.body.classList.add("platform-macos");
  }
}

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
