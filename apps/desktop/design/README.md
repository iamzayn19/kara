# Kara desktop — design reference

Static HTML mockups from the design pass before the Cursor-style rebuild,
kept for reference (not wired into the app — these are flat markup with
hardcoded sample content, not the real frontend).

Canvas: https://claude.ai/artifact/J1UkajB5NjUEDKeiDMrbx6

**Current / approved direction:**
- `Editor.dc.html` — file explorer, code editor with syntax highlighting,
  Claude-Code-style chat panel on the right (mic, attach, slash commands,
  session timer, permission-mode pill, circular send button)
- `Diff.dc.html` — same shell, center pane is a real side-by-side (not
  unified) diff view with per-file and all-files Accept/Reject
- `Voice.dc.html` — style guide: Supergirl/Kryptonian-themed status copy
  ("Flying in…", "X-raying the code…", "Stuck the landing.") and a
  Kryptonian crystal-blue accent for the activity timeline's neutral/active
  state. Semantic colors (green=success, red=error, amber=warning) are
  explicitly unchanged — only the neutral/idle tint and the flavor text.

**Superseded** (kept for history, not the direction being built):
- `Main.dc.html`, `Changes.dc.html` — first dark-theme pass, a chat-only
  window with a unified diff popup. Replaced once the ask became a full
  Cursor-style app (file tree + editor + side-by-side diff).
