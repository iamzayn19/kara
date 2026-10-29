//! End-to-end checks of the desktop app's file RPCs (fs/read, fs/write,
//! fs/search) against the real `kara serve --stdio` binary.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};

struct Rpc {
    child: Child,
    stdin: Option<ChildStdin>,
    /// Lines from the server, read on a thread so a request that never
    /// answers fails the test instead of hanging it.
    lines: Receiver<String>,
    next_id: u64,
}

impl Rpc {
    /// Starts the server in `workspace` with an isolated KARA_HOME pointed at
    /// a dummy endpoint, so no local model is ever looked for or downloaded.
    fn start(workspace: &Path, home: &Path) -> Self {
        Self::start_with_endpoint(workspace, home, "http://127.0.0.1:9/v1")
    }

    fn start_with_endpoint(workspace: &Path, home: &Path, endpoint: &str) -> Self {
        std::fs::write(
            home.join("config.toml"),
            format!("[inference]\nprovider = \"openai_compat\"\nendpoint = \"{endpoint}\"\nmodel = \"dummy\"\n"),
        )
        .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_kara"))
            .args(["serve", "--stdio"])
            .current_dir(workspace)
            .env("KARA_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut rpc = Rpc {
            child,
            stdin,
            lines,
            next_id: 0,
        };
        rpc.call("initialize", json!({ "workspace": workspace }))
            .expect("initialize");
        rpc
    }

    /// Sends a request without waiting; returns its id.
    fn send(&mut self, method: &str, params: Value) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        id
    }

    /// Sends a request and returns its result, or the error message.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.send(method, params);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let line = self.lines.recv_timeout(left).unwrap_or_else(|_| {
                panic!("{method}: no answer within 30s (or the server exited)")
            });
            let v: Value = serde_json::from_str(&line).unwrap();
            if v.get("id") != Some(&json!(id)) {
                continue; // notifications, logs, other requests' answers
            }
            return match v.get("error") {
                Some(e) => Err(e["message"].as_str().unwrap_or_default().to_string()),
                None => Ok(v["result"].clone()),
            };
        }
    }
}

impl Rpc {
    /// Closes our end of the pipe, as a quitting client does, and waits up
    /// to `timeout` for the server to exit.
    fn close_and_wait(&mut self, timeout: std::time::Duration) -> Option<std::process::ExitStatus> {
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        None
    }
}

impl Drop for Rpc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn project() -> (tempfile::TempDir, tempfile::TempDir) {
    let ws = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(ws.path())
        .status()
        .unwrap();
    std::fs::create_dir(ws.path().join("src")).unwrap();
    std::fs::write(ws.path().join("src/a.txt"), "hello\n").unwrap();
    (ws, home)
}

#[test]
fn save_round_trips_and_returns_the_new_version() {
    let (ws, home) = project();
    let mut rpc = Rpc::start(ws.path(), home.path());
    let read = rpc.call("fs/read", json!({"path": "src/a.txt"})).unwrap();
    assert_eq!(read["content"], "hello\n");

    let saved = rpc
        .call(
            "fs/write",
            json!({"path": "src/a.txt", "content": "edited\n", "expectedVersion": read["version"]}),
        )
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(ws.path().join("src/a.txt")).unwrap(),
        "edited\n"
    );
    let reread = rpc.call("fs/read", json!({"path": "src/a.txt"})).unwrap();
    assert_eq!(saved["version"], reread["version"]);
}

/// The file changes on disk between the editor's read and its save — even
/// within the same millisecond, which an mtime check let through.
#[test]
fn save_is_refused_when_the_file_changed_since_it_was_read() {
    let (ws, home) = project();
    let mut rpc = Rpc::start(ws.path(), home.path());
    let read = rpc.call("fs/read", json!({"path": "src/a.txt"})).unwrap();
    std::fs::write(ws.path().join("src/a.txt"), "kara changed this\n").unwrap();

    let err = rpc
        .call(
            "fs/write",
            json!({"path": "src/a.txt", "content": "stale\n", "expectedVersion": read["version"]}),
        )
        .unwrap_err();
    assert!(err.contains("changed on disk"), "{err}");
    assert_eq!(
        std::fs::read_to_string(ws.path().join("src/a.txt")).unwrap(),
        "kara changed this\n"
    );
}

#[test]
fn save_refuses_paths_outside_the_project_and_missing_files() {
    let (ws, home) = project();
    let outside = ws
        .path()
        .parent()
        .unwrap()
        .join(format!("kara-serve-fs-victim-{}", std::process::id()));
    std::fs::write(&outside, "secret").unwrap();
    let mut rpc = Rpc::start(ws.path(), home.path());

    let rel = format!("../{}", outside.file_name().unwrap().to_str().unwrap());
    let err = rpc
        .call("fs/write", json!({"path": rel, "content": "pwned"}))
        .unwrap_err();
    assert!(err.contains("escapes"), "{err}");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "secret");
    std::fs::remove_file(&outside).unwrap();

    assert!(rpc
        .call("fs/write", json!({"path": "src/new.txt", "content": "x"}))
        .is_err());
    assert!(!ws.path().join("src/new.txt").exists());
}

#[test]
fn search_is_literal_and_case_insensitive_by_default() {
    let (ws, home) = project();
    std::fs::write(
        ws.path().join("src/b.rs"),
        "fn Main() {}\nlet x = foo(1);\n",
    )
    .unwrap();
    let mut rpc = Rpc::start(ws.path(), home.path());

    let r = rpc.call("fs/search", json!({"query": "main"})).unwrap();
    let hits = r["matches"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["path"], "src/b.rs");
    assert_eq!(hits[0]["line"], 1);

    let r = rpc
        .call("fs/search", json!({"query": "main", "caseSensitive": true}))
        .unwrap();
    assert!(r["matches"].as_array().unwrap().is_empty());

    // Literal by default: the parenthesis is not a regex group.
    let r = rpc.call("fs/search", json!({"query": "foo("})).unwrap();
    assert_eq!(r["matches"].as_array().unwrap().len(), 1);

    let err = rpc
        .call("fs/search", json!({"query": "foo(", "regex": true}))
        .unwrap_err();
    assert!(err.contains("invalid pattern"), "{err}");

    assert!(rpc.call("fs/search", json!({"query": ""})).is_err());
}

#[test]
fn files_lists_the_project_respecting_gitignore() {
    let (ws, home) = project();
    std::fs::write(ws.path().join(".gitignore"), "build/\n").unwrap();
    std::fs::create_dir(ws.path().join("build")).unwrap();
    std::fs::write(ws.path().join("build/out.bin"), "x").unwrap();
    std::fs::write(ws.path().join("src/b.rs"), "x").unwrap();
    let mut rpc = Rpc::start(ws.path(), home.path());

    let r = rpc.call("fs/files", json!({})).unwrap();
    let files: Vec<&str> = r["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(files, vec![".gitignore", "src/a.txt", "src/b.rs"]);
    assert_eq!(r["truncated"], false);
}

/// When the editor or desktop app quits (or crashes), its end of the pipe
/// closes; the server must stop its model runtime and exit rather than
/// linger holding memory.
#[test]
fn server_exits_when_the_client_goes_away() {
    let (ws, home) = project();
    let mut rpc = Rpc::start(ws.path(), home.path());
    let status = rpc
        .close_and_wait(std::time::Duration::from_secs(10))
        .expect("kara serve still running 10s after its stdin closed");
    assert!(status.success(), "{status}");
}

#[test]
fn read_caps_large_files_and_gives_them_no_version() {
    let (ws, home) = project();
    // About 3 MB of text: over the 2 MB cap.
    let line = "0123456789abcdef".repeat(4) + "\n";
    let big = line.repeat(3 * 1024 * 1024 / line.len() + 1);
    std::fs::write(ws.path().join("big.log"), &big).unwrap();
    let mut rpc = Rpc::start(ws.path(), home.path());

    let r = rpc.call("fs/read", json!({"path": "big.log"})).unwrap();
    assert_eq!(r["truncated"], true);
    assert_eq!(r["content"].as_str().unwrap().len(), 2_000_000);
    // Truncated views are read-only: nothing to save back against.
    assert!(r["version"].is_null());

    let small = rpc.call("fs/read", json!({"path": "src/a.txt"})).unwrap();
    assert_eq!(small["truncated"], false);
    assert!(small["version"].is_string());
}

/// A running turn holds the session for its whole duration; the editor's
/// file requests must still answer meanwhile, or the desktop app's tree,
/// editor and search hang until Kara finishes.
#[test]
fn file_requests_answer_while_a_turn_is_running() {
    // A model server that accepts connections and never replies, so the
    // prompt below stays in flight.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for conn in listener.incoming() {
            held.push(conn);
        }
    });
    let (ws, home) = project();
    let mut rpc = Rpc::start_with_endpoint(
        ws.path(),
        home.path(),
        &format!("http://127.0.0.1:{port}/v1"),
    );
    rpc.send("session/prompt", json!({"text": "explain this project"}));
    std::thread::sleep(std::time::Duration::from_millis(500));
    // Proof the turn is really in flight (status refuses while one runs).
    let busy = |rpc: &mut Rpc| rpc.call("session/status", json!({})).unwrap_err();
    assert!(busy(&mut rpc).contains("busy"));

    for (method, params) in [
        ("fs/read", json!({"path": "src/a.txt"})),
        ("fs/list", json!({"path": ""})),
        ("fs/files", json!({})),
        ("fs/search", json!({"query": "hello"})),
    ] {
        let t0 = std::time::Instant::now();
        rpc.call(method, params)
            .unwrap_or_else(|e| panic!("{method}: {e}"));
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(2),
            "{method} waited {:?} for the running turn",
            t0.elapsed()
        );
    }
    assert!(busy(&mut rpc).contains("busy"), "the turn ended early");
}

#[test]
fn create_and_rename_stay_inside_and_never_overwrite() {
    let (ws, home) = project();
    let mut rpc = Rpc::start(ws.path(), home.path());

    rpc.call("fs/create", json!({"path": "src/new.rs"}))
        .unwrap();
    assert_eq!(std::fs::read(ws.path().join("src/new.rs")).unwrap(), b"");
    rpc.call("fs/create", json!({"path": "docs", "kind": "dir"}))
        .unwrap();
    assert!(ws.path().join("docs").is_dir());

    // Never overwrite: an existing file keeps its content.
    let err = rpc
        .call("fs/create", json!({"path": "src/a.txt"}))
        .unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    assert_eq!(
        std::fs::read_to_string(ws.path().join("src/a.txt")).unwrap(),
        "hello\n"
    );

    // Nothing outside the workspace, by .. or by a missing parent.
    for bad in [
        "../escape.txt",
        "src/../../escape.txt",
        "nope/x.txt",
        "src/..",
        "",
    ] {
        assert!(
            rpc.call("fs/create", json!({"path": bad})).is_err(),
            "{bad}"
        );
    }
    assert!(!ws.path().parent().unwrap().join("escape.txt").exists());

    rpc.call(
        "fs/rename",
        json!({"from": "src/new.rs", "to": "docs/moved.rs"}),
    )
    .unwrap();
    assert!(ws.path().join("docs/moved.rs").is_file());
    assert!(!ws.path().join("src/new.rs").exists());
    let err = rpc
        .call(
            "fs/rename",
            json!({"from": "docs/moved.rs", "to": "src/a.txt"}),
        )
        .unwrap_err();
    assert!(err.contains("already exists"), "{err}");
    assert!(rpc
        .call("fs/rename", json!({"from": "src/a.txt", "to": "../a.txt"}))
        .is_err());
    assert!(rpc
        .call("fs/rename", json!({"from": "", "to": "x"}))
        .is_err());

    // Renaming a symlink renames the link, not the file it points to.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("a.txt", ws.path().join("src/link")).unwrap();
        rpc.call("fs/rename", json!({"from": "src/link", "to": "src/link2"}))
            .unwrap();
        assert!(ws.path().join("src/a.txt").is_file());
        assert!(ws
            .path()
            .join("src/link2")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
    }
}

#[test]
fn trash_refuses_anything_outside_the_workspace() {
    let (ws, home) = project();
    let outside = ws
        .path()
        .parent()
        .unwrap()
        .join(format!("kara-trash-victim-{}", std::process::id()));
    std::fs::write(&outside, "keep me").unwrap();
    let mut rpc = Rpc::start(ws.path(), home.path());
    let rel = format!("../{}", outside.file_name().unwrap().to_str().unwrap());
    for bad in [rel.as_str(), "src/..", "", "nope.txt"] {
        assert!(
            rpc.call("fs/trash", json!({"path": bad})).is_err(),
            "{bad:?}"
        );
    }
    assert!(outside.exists());
    std::fs::remove_file(&outside).unwrap();
    assert!(ws.path().join("src/a.txt").exists());
}

/// Really moves a file to the OS trash, so it only runs on CI (GitHub sets
/// CI=true) rather than filling a developer's Trash on every test run.
#[test]
fn trash_moves_a_file_out_of_the_workspace() {
    if std::env::var_os("CI").is_none() {
        return;
    }
    let (ws, home) = project();
    let name = format!("kara-trash-test-{}.txt", std::process::id());
    std::fs::write(ws.path().join(&name), "bye").unwrap();
    let mut rpc = Rpc::start(ws.path(), home.path());
    rpc.call("fs/trash", json!({"path": name})).unwrap();
    assert!(!ws.path().join(&name).exists());
    assert!(ws.path().join("src/a.txt").exists());
}

#[test]
fn read_flags_text_that_is_not_utf8() {
    let (ws, home) = project();
    // "café" in Latin-1: 0xE9 is not valid UTF-8 on its own.
    std::fs::write(ws.path().join("latin1.txt"), b"caf\xe9\n").unwrap();
    let mut rpc = Rpc::start(ws.path(), home.path());
    let r = rpc.call("fs/read", json!({"path": "latin1.txt"})).unwrap();
    assert_eq!(r["utf8"], false);
    let r = rpc.call("fs/read", json!({"path": "src/a.txt"})).unwrap();
    assert_eq!(r["utf8"], true);
}
