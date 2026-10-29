// A real shell panel, backed by a PTY in the Rust side (src-tauri/src/terminal.rs),
// not a sandboxed fake — the same kind of integrated terminal Cursor/VSCode have.

import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const THEMES = {
  dark: { background: "#0a0b0f", foreground: "#ececf2", cursor: "#ef4b55", cursorAccent: "#0a0b0f", selectionBackground: "#5b8cff44" },
  light: { background: "#eef0f4", foreground: "#16171d", cursor: "#d42f3a", cursorAccent: "#eef0f4", selectionBackground: "#2f62d933" },
};

let term: Terminal | undefined;
let fitAddon: FitAddon | undefined;
let spawned = false;
/** Id of the running shell, and the highest id that was stopped on
 * purpose: events from those are ignored. Filtering on "not stopped"
 * rather than "is current" keeps a new shell's first output, which can
 * arrive before terminal_spawn returns its id. */
let currentId = 0;
let stoppedUpTo = 0;

export async function openTerminal(container: HTMLElement, cwd: string, theme: "light" | "dark") {
  if (!term) {
    term = new Terminal({
      fontSize: 12.5,
      fontFamily: "ui-monospace, 'SF Mono', Menlo, monospace",
      theme: THEMES[theme],
      cursorBlink: true,
    });
    fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(container);
    term.onData((data) => {
      invoke("terminal_write", { data }).catch(() => {});
    });

    await listen<{ id: number; data: string }>("kara://terminal-output", (e) => {
      if (e.payload.id > stoppedUpTo) term?.write(e.payload.data);
    });
    await listen<{ id: number }>("kara://terminal-exit", (e) => {
      if (e.payload.id <= stoppedUpTo) return;
      term?.write("\r\n[process exited — reopen the panel for a new shell]\r\n");
      spawned = false;
    });
  }
  fitAddon?.fit();
  if (!spawned) {
    spawned = true;
    try {
      currentId = await invoke<number>("terminal_spawn", { cwd });
    } catch (e) {
      spawned = false;
      term?.write(`\r\nfailed to start shell: ${e}\r\n`);
    }
    reportSize();
  }
}

export function fitTerminal() {
  fitAddon?.fit();
  reportSize();
}

function reportSize() {
  if (!term) return;
  invoke("terminal_resize", { rows: term.rows, cols: term.cols }).catch(() => {});
}

export function setTerminalTheme(theme: "light" | "dark") {
  if (term) term.options.theme = THEMES[theme];
}

/** Stops the shell (e.g. the project changed) and clears the screen; the
 * next openTerminal starts a fresh one in its cwd. */
export async function stopTerminal() {
  if (!spawned) return;
  spawned = false;
  stoppedUpTo = Math.max(stoppedUpTo, currentId);
  term?.reset();
  await invoke("terminal_kill").catch(() => {});
}
