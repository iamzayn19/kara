// Kara chat webview. Renders protocol events; never runs agent logic.
(function () {
  const vscode = acquireVsCodeApi();
  const log = document.getElementById("log");
  const input = document.getElementById("input");
  const working = document.getElementById("working");
  const phaseEl = document.getElementById("phase");
  const modelEl = document.getElementById("model");
  const profileEl = document.getElementById("profile");
  let current = null; // element receiving streamed assistant text
  let thinking = null;
  const tools = new Map();

  const PHASES = {
    understand: "understanding the task",
    search: "searching the repository",
    read: "reading code",
    plan: "planning",
    edit: "editing",
    test: "running tests",
    recover: "tests failed; retrying",
    verify: "verifying",
    review: "reviewing the diff",
    summarize: "summarizing",
  };

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  function scroll() {
    log.scrollTop = log.scrollHeight;
  }

  function add(node) {
    log.appendChild(node);
    scroll();
    return node;
  }

  function endStream() {
    current = null;
    if (thinking) {
      thinking.remove();
      thinking = null;
    }
  }

  function assistantText(text) {
    if (!current) {
      current = add(el("div", "msg assistant"));
      current.dataset.raw = "";
    }
    current.dataset.raw += text;
    // Hide tool-call markup some models stream as text.
    const raw = current.dataset.raw;
    const cut = raw.indexOf("<tool_call>");
    current.textContent = cut >= 0 ? raw.slice(0, cut) : raw;
    scroll();
  }

  function renderDiff(diff) {
    const pre = el("pre", "diff");
    for (const line of diff.split("\n")) {
      if (line.startsWith("+++") || line.startsWith("---")) continue;
      const cls = line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : line.startsWith("@@") ? "hunk" : "ctx";
      pre.appendChild(el("div", cls, line || " "));
    }
    return pre;
  }

  function button(label, onClick, cls) {
    const b = el("button", cls || "", label);
    b.addEventListener("click", onClick);
    return b;
  }

  function handleEvent(e) {
    switch (e.type) {
      case "turn_started":
        tools.clear();
        break;
      case "phase":
        phaseEl.textContent = PHASES[e.phase] || e.phase;
        if (e.phase === "recover") add(el("div", "notice warn", "Tests failed; investigating and retrying."));
        break;
      case "reasoning_delta":
        if (!thinking && !current) thinking = add(el("div", "thinking", "thinking…"));
        break;
      case "assistant_delta":
        if (thinking) {
          thinking.remove();
          thinking = null;
        }
        assistantText(e.text);
        break;
      case "assistant_message":
        if (!current) assistantText(e.text);
        endStream();
        break;
      case "tool_started": {
        endStream();
        const row = add(el("div", "tool running"));
        row.appendChild(el("span", "arrow", "→"));
        row.appendChild(el("span", "title", e.summary));
        tools.set(e.call_id, row);
        break;
      }
      case "tool_finished": {
        const row = tools.get(e.call_id);
        if (row) {
          row.classList.remove("running");
          row.classList.add(e.ok ? "ok" : "fail");
          row.appendChild(el("span", "result", (e.ok ? "✓ " : "✗ ") + e.summary));
        }
        break;
      }
      case "file_changed": {
        endStream();
        const c = e.change;
        const card = add(el("div", "change"));
        const head = el("div", "change-head");
        const name = el("a", "path", c.path);
        name.addEventListener("click", () => vscode.postMessage({ type: "openFile", path: c.path }));
        head.appendChild(name);
        head.appendChild(el("span", "kind", c.kind));
        head.appendChild(button("View diff", () => vscode.postMessage({ type: "viewDiff", path: c.path })));
        const accept = button("Accept", () => {
          card.classList.add("accepted");
          accept.disabled = true;
          accept.textContent = "Accepted";
        });
        head.appendChild(accept);
        head.appendChild(button("Reject", () => vscode.postMessage({ type: "reject", path: c.path }), "secondary"));
        card.appendChild(head);
        const lines = c.diff.split("\n").length;
        if (lines < 40) card.appendChild(renderDiff(c.diff));
        break;
      }
      case "test_finished": {
        endStream();
        const r = e.report;
        const ok = r.exit_code === 0;
        const counts = r.passed != null && r.failed != null ? `${r.passed} passed, ${r.failed} failed` : `exit ${r.exit_code}`;
        const box = add(el("div", "tests " + (ok ? "ok" : "fail"), (ok ? "Tests passed: " : "Tests failed: ") + counts));
        for (const t of (r.failed_tests || []).slice(0, 8)) box.appendChild(el("div", "failed-test", "✗ " + t));
        break;
      }
      case "plan_updated": {
        endStream();
        const box = add(el("div", "plan"));
        for (const s of e.steps) {
          const mark = { done: "[x]", in_progress: "[>]", skipped: "[-]", pending: "[ ]" }[s.status] || "[ ]";
          box.appendChild(el("div", "step " + s.status, `${mark} ${s.title}`));
        }
        if (e.hypothesis) box.appendChild(el("div", "hypothesis", "Hypothesis: " + e.hypothesis));
        break;
      }
      case "notice":
        add(el("div", "notice " + (e.level === "info" ? "" : e.level === "warning" ? "warn" : "err"), e.message));
        break;
      case "turn_finished": {
        endStream();
        if (e.outcome === "awaiting_approval") {
          const box = add(el("div", "approve"));
          box.appendChild(el("span", "", "Plan ready. Approve to carry it out, or reply with changes."));
          box.appendChild(button("Approve plan", () => vscode.postMessage({ type: "approve" })));
        } else if (e.outcome !== "completed") {
          add(el("div", "notice warn", `Task ${e.outcome.replace("_", " ")}.`));
        }
        if (e.changed_files && e.changed_files.length) {
          add(el("div", "summary", "Changed: " + e.changed_files.join(", ")));
        }
        const u = e.usage || {};
        if ((u.prompt_tokens || 0) + (u.completion_tokens || 0) > 0) {
          add(el("div", "usage", `${u.prompt_tokens} tokens in · ${u.completion_tokens} out`));
        }
        break;
      }
    }
  }

  function renderCommand(name, r) {
    const pre = el("pre", "command-output");
    pre.textContent = JSON.stringify(r, null, 2);
    const box = add(el("div", "command"));
    box.appendChild(el("div", "command-title", name));
    box.appendChild(pre);
  }

  window.addEventListener("message", (ev) => {
    const m = ev.data;
    switch (m.type) {
      case "event":
        handleEvent(m.event);
        break;
      case "user":
        endStream();
        add(el("div", "msg user" + (m.mode && m.mode !== "execute" ? " " + m.mode : ""), m.text));
        break;
      case "busy":
        working.hidden = !m.busy;
        input.disabled = false;
        if (!m.busy) endStream();
        break;
      case "status":
        modelEl.textContent = m.running ? (m.modelAvailable ? m.model : "no model, choose one") : "agent not running";
        modelEl.classList.toggle("warn", !m.modelAvailable || !m.running);
        profileEl.textContent = m.permissionsMode ? "permissions: " + m.permissionsMode : "";
        break;
      case "info":
        add(el("div", "notice", m.text));
        break;
      case "error": {
        endStream();
        const box = add(el("div", "notice err", m.text));
        if (m.action === "chooseModel") box.appendChild(button("Choose model", () => vscode.postMessage({ type: "chooseModel" })));
        break;
      }
      case "result":
        break;
      case "guidance": {
        endStream();
        const box = add(el("div", "guidance"));
        box.appendChild(el("div", "", m.text));
        const row = el("div", "actions");
        row.appendChild(button("Choose local model", () => vscode.postMessage({ type: "chooseModel" })));
        row.appendChild(button("Connect to a Kara machine", () => vscode.postMessage({ type: "connect" }), "secondary"));
        box.appendChild(row);
        break;
      }
      case "command":
        renderCommand(m.name, m.result);
        break;
      case "help": {
        const box = add(el("div", "help"));
        box.appendChild(el("div", "", "Talk to Kara in plain language. Commands:"));
        for (const c of m.commands) box.appendChild(el("div", "help-row", `${c.name}  ${c.description}`));
        break;
      }
    }
  });

  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      const text = input.value;
      if (text.trim()) {
        vscode.postMessage({ type: "send", text });
        input.value = "";
      }
    }
  });
  document.getElementById("stop").addEventListener("click", () => vscode.postMessage({ type: "stop" }));
  vscode.postMessage({ type: "ready" });
})();
