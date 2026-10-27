// A real shell panel, backed by a PTY in the Rust side (src-tauri/src/terminal.rs),
// not a sandboxed fake — the same kind of integrated terminal Cursor/VSCode have.

import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

let term: Terminal | undefined;
let fitAddon: FitAddon | undefined;
let spawned = false;
let unlistenOutput: (() => void) | undefined;
let unlistenExit: (() => void) | undefined;

export async function openTerminal(container: HTMLElement, cwd: string) {
  if (!term) {
    term = new Terminal({
      fontSize: 12.5,
      fontFamily: "ui-monospace, 'SF Mono', Menlo, monospace",
      theme: { background: "#0a0b0f", foreground: "#ececf2", cursor: "#ef4b55", selectionBackground: "#5b8cff44" },
      cursorBlink: true,
    });
    fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(container);
    term.onData((data) => {
      invoke("terminal_write", { data }).catch(() => {});
    });

    unlistenOutput = await listen<string>("kara://terminal-output", (e) => {
      term?.write(e.payload);
    });
    unlistenExit = await listen("kara://terminal-exit", () => {
      term?.write("\r\n[process exited — reopen the panel for a new shell]\r\n");
      spawned = false;
    });
  }
  fitAddon?.fit();
  if (!spawned) {
    spawned = true;
    await invoke("terminal_spawn", { cwd }).catch((e) => {
      term?.write(`\r\nfailed to start shell: ${e}\r\n`);
    });
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

export function disposeTerminal() {
  unlistenOutput?.();
  unlistenExit?.();
  term?.dispose();
  term = undefined;
  fitAddon = undefined;
  spawned = false;
}
