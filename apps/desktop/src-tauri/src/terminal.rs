//! A single interactive PTY-backed terminal session per app window.
//! Uses a real pseudo-terminal (not a piped child process) so the shell
//! inside it behaves exactly like a terminal app's own: line editing,
//! color, job control, and resize all work because the shell is actually
//! talking to a tty, not a pipe.

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

#[derive(Default)]
pub struct Terminal {
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
}

fn default_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "powershell.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
    }
}

impl Terminal {
    pub fn spawn(&self, app: AppHandle, cwd: String) -> Result<(), String> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
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

        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                        let _ = app.emit("kara://terminal-output", chunk);
                    }
                    Err(_) => break,
                }
            }
            let _ = child.wait();
            let _ = app.emit("kara://terminal-exit", ());
        });

        Ok(())
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
                .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
                .map_err(|e| e.to_string()),
            None => Err("no terminal session running".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
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
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut cmd = if cfg!(windows) { CommandBuilder::new("cmd.exe") } else { CommandBuilder::new("/bin/sh") };
        cmd.cwd(std::env::temp_dir());
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();

        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 || tx.send(String::from_utf8_lossy(&buf[..n]).into_owned()).is_err() {
                    break;
                }
            }
        });

        // Arithmetic so the match can only come from the shell's output, not
        // from the tty echoing our own input back.
        writer.write_all(b"echo kara-$((40+2))\r").unwrap();

        let mut collected = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !collected.contains("kara-42") && Instant::now() < deadline {
            if let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
                collected.push_str(&chunk);
            }
        }
        let _ = child.kill();

        assert!(collected.contains("kara-42"), "shell never ran the command; saw: {collected:?}");
    }
}
