//! Every fixture task must fail before its fix and pass after applying the
//! reference solution with Kara's own patch tool. This keeps the evaluation
//! suite honest: a task that already passes, or whose solution does not work,
//! is a broken benchmark.

use serde_json::json;
use std::sync::{Arc, Mutex};
use kara_integration_tests::*;
use kara_tools::{Journal, Tool, ToolContext};

fn toolchain_for(fixture: &str) -> Option<(&'static str, &'static str)> {
    match fixture {
        "python-shop" => Some((python(), "--version")),
        "ruby-auth" => Some(("ruby", "--version")),
        "typescript-pagination" => Some(("node", "--version")),
        "rust-ledger" => Some(("cargo", "--version")),
        "go-inventory" => Some(("go", "version")),
        "java-orders" => Some(("javac", "-version")),
        _ => None,
    }
}

fn node_supports_type_stripping() -> bool {
    let out = std::process::Command::new("node").arg("--version").output();
    let Ok(out) = out else { return false };
    let v = String::from_utf8_lossy(&out.stdout);
    let major: u32 = v
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|m| m.parse().ok())
        .unwrap_or(0);
    major >= 23
}

async fn apply_solution(repo: &Repo, fixture: &str, task: &str) {
    let patch = std::fs::read_to_string(
        fixtures_dir()
            .join(fixture)
            .join("solutions")
            .join(format!("{task}.patch")),
    )
    .unwrap();
    let state = tempfile::tempdir().unwrap();
    let ws = kara_sandbox::Workspace::new(repo.path()).unwrap();
    let ctx = ToolContext {
        profile: kara_context::ProjectProfile::default(),
        journal: Arc::new(Mutex::new(Journal::open(ws.root(), state.path()).unwrap())),
        workspace: ws,
        index: None,
        output_chars: 8000,
        cancel: Default::default(),
        allow_commands: vec![],
        deny_commands: vec![],
    };
    let out = kara_tools::fs::ApplyPatch
        .run(&json!({"patch": patch}), &ctx)
        .await;
    assert!(
        out.ok,
        "solution for {task} does not apply: {}",
        out.content
    );
}

#[tokio::test]
async fn every_fixture_task_fails_before_and_passes_after_its_solution() {
    let mut checked = 0;
    let mut skipped = Vec::new();
    for entry in std::fs::read_dir(fixtures_dir()).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        let Some((tool, arg)) = toolchain_for(&name) else {
            continue;
        };
        if !have(tool, arg) || (name == "typescript-pagination" && !node_supports_type_stripping())
        {
            skipped.push(name);
            continue;
        }
        if name == "java-orders" && (cfg!(windows) || !have("make", "--version")) {
            skipped.push(name);
            continue;
        }
        for task in fixture_tasks(&name) {
            let repo = Repo::from_fixture(&name);
            for dep in &task.requires {
                apply_solution(&repo, &name, dep).await;
            }
            let (ok, out) = repo.run(task.check_command());
            assert!(
                !ok,
                "{}: check passes before the fix (broken benchmark):\n{out}",
                task.id
            );
            apply_solution(&repo, &name, &task.id).await;
            let (ok, out) = repo.run(task.check_command());
            assert!(
                ok,
                "{}: check still fails after the reference solution:\n{out}",
                task.id
            );
            checked += 1;
        }
    }
    eprintln!("checked {checked} fixture tasks; skipped (toolchain missing): {skipped:?}");
    assert!(
        checked >= 2,
        "at least the Python and one other fixture must run"
    );
}

#[test]
fn project_commands_are_detected_for_fixtures() {
    let reg = kara_context::LanguageRegistry::builtin();
    let expect = [
        ("python-shop", "unittest"),
        ("ruby-auth", "ruby -Itest -Ilib"),
        ("typescript-pagination", "npm test"),
        ("rust-ledger", "cargo test"),
        ("go-inventory", "go test ./..."),
        ("java-orders", "make test"),
    ];
    for (name, needle) in expect {
        let p = kara_context::ProjectProfile::detect(&fixtures_dir().join(name), &reg);
        let test = p
            .first(kara_context::CommandCategory::Test)
            .unwrap_or_else(|| panic!("{name}: no test command"));
        assert!(test.run.contains(needle), "{name}: {}", test.run);
    }
}
