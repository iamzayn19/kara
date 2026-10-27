//! End-to-end checks of the desktop app's file RPCs (fs/read, fs/write,
//! fs/search) against the real `kara serve --stdio` binary.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Rpc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Rpc {
    /// Starts the server in `workspace` with an isolated KARA_HOME pointed at
    /// a dummy endpoint, so no local model is ever looked for or downloaded.
    fn start(workspace: &Path, home: &Path) -> Self {
        std::fs::write(
            home.join("config.toml"),
            "[inference]\nprovider = \"openai_compat\"\nendpoint = \"http://127.0.0.1:9/v1\"\nmodel = \"dummy\"\n",
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
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut rpc = Rpc {
            child,
            stdin,
            stdout,
            next_id: 0,
        };
        rpc.call("initialize", json!({ "workspace": workspace }))
            .expect("initialize");
        rpc
    }

    /// Sends a request and returns its result, or the error message.
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).unwrap() > 0,
                "server exited"
            );
            let v: Value = serde_json::from_str(&line).unwrap();
            if v.get("id") != Some(&json!(id)) {
                continue; // notifications and logs
            }
            return match v.get("error") {
                Some(e) => Err(e["message"].as_str().unwrap_or_default().to_string()),
                None => Ok(v["result"].clone()),
            };
        }
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
