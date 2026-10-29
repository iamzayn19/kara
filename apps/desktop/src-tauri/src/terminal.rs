//! A single interactive PTY-backed terminal session per app window.
//! Uses a real pseudo-terminal (not a piped child process) so the shell
//! inside it behaves exactly like a terminal app's own: line editing,
//! color, job control, and resize all work because the shell is actually
//! talking to a tty, not a pipe.

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

#[derive(Default)]
pub struct Terminal {
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
    killer: Mutex<Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    /// Id of the current shell. Output and exit events carry it so the
    /// window can ignore a shell that was replaced (e.g. on project switch).
    generation: AtomicU64,
}

fn default_shell() -> String {
    if cfg!(windows) {
        let path = std::env::var("PATH").unwrap_or_default();
        windows_shell(&path, |p| p.is_file())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
    }
}

/// PowerShell 7 if installed, else the Windows PowerShell every Windows
/// ships, like VS Code and Windows Terminal; cmd.exe only as a last resort.
fn windows_shell(path_var: &str, exists: impl Fn(&std::path::Path) -> bool) -> String {
    for name in ["pwsh.exe", "powershell.exe"] {
        for dir in path_var.split(';').filter(|d| !d.is_empty()) {
            let candidate = std::path::Path::new(dir).join(name);
            if exists(&candidate) {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
}

impl Terminal {
    /// Starts a shell in `cwd`, replacing any running one; returns its id.
    pub fn spawn(&self, app: AppHandle, cwd: String) -> Result<u64, String> {
        self.kill();
        let id = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;

        let mut cmd = CommandBuilder::new(default_shell());
        cmd.cwd(cwd);

        let mut child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
        // Keeping our copy of the slave open would stop the reader from ever
        // seeing EOF when the shell exits.
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;

        *self.writer.lock().unwrap() = Some(writer);
        *self.master.lock().unwrap() = Some(pair.master);
        *self.killer.lock().unwrap() = Some(child.clone_killer());

        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut pending = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        let chunk = take_utf8(&mut pending);
                        if !chunk.is_empty() {
                            let _ = app.emit(
                                "kara://terminal-output",
                                serde_json::json!({ "id": id, "data": chunk }),
                            );
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = child.wait();
            let _ = app.emit("kara://terminal-exit", serde_json::json!({ "id": id }));
        });

        Ok(id)
    }

    /// Stops the running shell, if any.
    pub fn kill(&self) {
        if let Some(mut k) = self.killer.lock().unwrap().take() {
            let _ = k.kill();
        }
        self.writer.lock().unwrap().take();
        self.master.lock().unwrap().take();
    }

    pub fn write(&self, data: &str) -> Result<(), String> {
        let mut guard = self.writer.lock().unwrap();
        match guard.as_mut() {
            Some(w) => w.write_all(data.as_bytes()).map_err(|e| e.to_string()),
            None => Err("no terminal session running".to_string()),
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), String> {
        let guard = self.master.lock().unwrap();
        match guard.as_ref() {
            Some(m) => m
                .resize(PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| e.to_string()),
            None => Err("no terminal session running".to_string()),
        }
    }
}

/// Decodes as much of `pending` as forms complete UTF-8, leaving a
/// character split across reads in place for the next one. Invalid bytes
/// become U+FFFD as before.
fn take_utf8(pending: &mut Vec<u8>) -> String {
    let mut out = String::new();
    loop {
        match std::str::from_utf8(pending) {
            Ok(s) => {
                out.push_str(s);
                pending.clear();
                return out;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                out.push_str(std::str::from_utf8(&pending[..valid]).unwrap());
                match e.error_len() {
                    // Incomplete sequence at the end: wait for more bytes.
                    None => {
                        pending.drain(..valid);
                        return out;
                    }
                    Some(bad) => {
                        out.push(char::REPLACEMENT_CHARACTER);
                        pending.drain(..valid + bad);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{take_utf8, windows_shell};

    #[test]
    fn windows_shell_prefers_pwsh_then_windows_powershell() {
        let path = r"C:\Windows\System32;C:\Windows\System32\WindowsPowerShell\v1.0;C:\Program Files\PowerShell\7";
        let only = |names: &'static [&'static str]| {
            move |p: &std::path::Path| names.iter().any(|n| p.to_string_lossy().ends_with(n))
        };
        assert!(windows_shell(
            path,
            only(&[
                "7/pwsh.exe",
                "7\\pwsh.exe",
                r"v1.0/powershell.exe",
                r"v1.0\powershell.exe"
            ])
        )
        .ends_with("pwsh.exe"));
        assert!(windows_shell(
            path,
            only(&[r"v1.0/powershell.exe", r"v1.0\powershell.exe"])
        )
        .ends_with("powershell.exe"));
        let fallback = windows_shell(path, |_| false);
        assert!(fallback.to_lowercase().ends_with("cmd.exe"), "{fallback}");
    }

    #[test]
    fn utf8_split_across_reads_is_not_mangled() {
        let text = "ok ✓ کارا ─┐ 🚀";
        let bytes = text.as_bytes();
        // Every possible split point, including inside each multi-byte char.
        for cut in 0..=bytes.len() {
            let mut pending = bytes[..cut].to_vec();
            let mut got = take_utf8(&mut pending);
            pending.extend_from_slice(&bytes[cut..]);
            got.push_str(&take_utf8(&mut pending));
            assert_eq!(got, text, "split at byte {cut}");
            assert!(pending.is_empty());
        }
    }

    #[test]
    fn invalid_bytes_still_become_replacement_chars() {
        let mut pending = b"a\xffb".to_vec();
        assert_eq!(take_utf8(&mut pending), "a\u{fffd}b");
        assert!(pending.is_empty());
    }

    use portable_pty::{native_pty_system, CommandBuilder, PtySize};
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    // Same openpty/spawn/read/write sequence Terminal::spawn uses, minus the
    // Tauri AppHandle. Reads run on their own thread so a silent shell fails
    // the test instead of hanging it.
    #[test]
    fn pty_runs_a_real_shell_command() {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = if cfg!(windows) {
            CommandBuilder::new("cmd.exe")
        } else {
            CommandBuilder::new("/bin/sh")
        };
        cmd.cwd(std::env::temp_dir());
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();

        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0
                    || tx
                        .send(String::from_utf8_lossy(&buf[..n]).into_owned())
                        .is_err()
                {
                    break;
                }
            }
        });

        // Arithmetic so the match can only come from the shell's output, not
        // from the tty echoing our own input back. cmd.exe has no $((...)).
        let (input, expected): (&[u8], &str) = if cfg!(windows) {
            (b"set /a 6*7\r\n", "42")
        } else {
            (b"echo kara-$((6*7))\r", "kara-42")
        };
        writer.write_all(input).unwrap();

        let mut collected = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !collected.contains(expected) && Instant::now() < deadline {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
                collected.push_str(&chunk);
            }
        }
        let _ = child.kill();

        assert!(
            collected.contains(expected),
            "shell never ran the command; saw: {collected:?}"
        );
    }
}
