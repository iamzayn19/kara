// Kara desktop: a thin window over `kara serve --stdio`. All agent logic
// lives in the Rust core; this file only renders events and sends prompts,
// mirroring the VS Code extension's chatView.ts for the same protocol.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

let busy = false;
let pendingPlan = false;
let spinnerTimer: number | undefined;
let spinnerRow: HTMLElement | undefined;
let spinnerFrame = 0;
const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

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

async function send() {
  if (busy) {
    await cancel();
    return;
  }
  const text = input.value.trim();
  if (!text) return;
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
    transcript.querySelectorAll(".bubble-row, .activity, .diff-block").forEach((el) => el.remove());
    emptyState.style.display = "flex";
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

function autosize() {
  input.style.height = "auto";
  input.style.height = `${Math.min(input.scrollHeight, 160)}px`;
}

input.addEventListener("input", autosize);
input.addEventListener("keydown", (ev) => {
  if (ev.key === "Enter" && !ev.shiftKey && !ev.metaKey && !ev.ctrlKey) {
    ev.preventDefault();
    send();
  } else if (ev.key === "Escape" && busy) {
    cancel();
  }
});
sendBtn.addEventListener("click", send);
btnNew.addEventListener("click", newSession);
btnUndo.addEventListener("click", undo);

async function boot() {
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

  sessionState.textContent = "starting…";
  // v1: no folder picker yet — defaults to the user's home directory on
  // the Rust side (empty string). A real "open project" flow is the next
  // step before this is a general-purpose app rather than a single fixed
  // workspace.
  try {
    const info: any = await invoke("kara_start", { workspace: "" });
    modelPill.textContent = info?.model?.available ? info.model.label : "no model";
    modelPill.className = info?.model?.available ? "pill ready" : "pill warn";
    permPill.textContent = info?.permissionsMode ?? "—";
    sessionState.textContent = "ready";
  } catch (e: any) {
    activity(`Kara failed to start: ${e?.message ?? e}`, "err");
    sessionState.textContent = "not running";
  }
}

boot();
