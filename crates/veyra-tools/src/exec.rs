//! Process tools: shell, tests, lint/typecheck, build, diagnostics.

use crate::output::for_model;
use crate::process::{run_shell, ProcessResult};
use crate::testparse;
use crate::worktree::WorktreeSnapshot;
use crate::{arg_str, opt_str, opt_str_list, opt_u64, Assessment, Tool, ToolContext, ToolOutput};
use regex::Regex;
use serde_json::{json, Value};
use std::time::Duration;
use veyra_context::project::expand_targeted;
use veyra_context::CommandCategory;
use veyra_protocol::{ActionKind, TestReport};
use veyra_sandbox::{classify_command, CommandContext};

fn classify(cmd: &str, ctx: &ToolContext) -> Assessment {
    let known = ctx.known_commands();
    let cc = CommandContext {
        workspace: &ctx.workspace,
        known: &known,
        user_allow: &ctx.allow_commands,
        user_deny: &ctx.deny_commands,
    };
    let c = classify_command(cmd, &cc);
    Assessment {
        kinds: c.kinds_vec(),
        title: format!("run: {cmd}"),
        detail: cmd.to_string(),
        reasons: c.reasons,
        blocked: c.blocked,
        user_allowed: c.user_allowed,
    }
}

/// Whether a command might modify the worktree (so we journal its effects).
fn may_write(kinds: &[ActionKind]) -> bool {
    kinds
        .iter()
        .any(|k| !matches!(k, ActionKind::Read | ActionKind::Search | ActionKind::GitRead))
}

async fn execute(cmd: &str, ctx: &ToolContext, timeout: Duration, kinds: &[ActionKind]) -> anyhow::Result<(ProcessResult, String)> {
    let snapshot = if may_write(kinds) {
        let root = ctx.root().to_path_buf();
        Some(tokio::task::spawn_blocking(move || WorktreeSnapshot::capture(&root)).await?)
    } else {
        None
    };
    let result = run_shell(cmd, ctx.root(), timeout, &ctx.cancel).await?;
    let mut note = String::new();
    if let Some(snap) = snapshot {
        let changes = {
            let mut j = ctx.journal();
            snap.journal_changes(ctx.root(), &mut j)
        };
        if !changes.journaled.is_empty() {
            note.push_str(&format!(
                "files changed by this command (undoable): {}\n",
                changes.journaled.join(", ")
            ));
        }
        if !changes.not_undoable.is_empty() {
            note.push_str(&format!(
                "files changed by this command that /undo cannot restore (they had uncommitted changes Veyra had not snapshotted): {}\n",
                changes.not_undoable.join(", ")
            ));
        }
    }
    Ok((result, note))
}

fn format_result(r: &ProcessResult, note: &str, ctx: &ToolContext, extra: &str) -> String {
    let status = if r.timed_out {
        "TIMED OUT".to_string()
    } else if r.cancelled {
        "CANCELLED".to_string()
    } else {
        format!("exit code {}", r.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "?".into()))
    };
    let mut s = format!("$ {}\n{status} after {:.1}s\n{extra}{note}", r.command, r.duration_ms as f64 / 1000.0);
    let budget = ctx.output_chars;
    let (out_budget, err_budget) = if r.stderr.trim().is_empty() {
        (budget, 0)
    } else if r.stdout.trim().is_empty() {
        (0, budget)
    } else {
        (budget / 2, budget / 2)
    };
    if !r.stdout.trim().is_empty() {
        s.push_str("--- stdout ---\n");
        s.push_str(&for_model(&r.stdout, out_budget));
        if !s.ends_with('\n') {
            s.push('\n');
        }
    }
    if !r.stderr.trim().is_empty() {
        s.push_str("--- stderr ---\n");
        s.push_str(&for_model(&r.stderr, err_budget));
    }
    s
}

fn timeout_arg(args: &Value, default: u64) -> Duration {
    Duration::from_secs(opt_u64(args, "timeout_secs").unwrap_or(default).clamp(1, 4 * 3600))
}

pub struct Shell;

#[async_trait::async_trait]
impl Tool for Shell {
    fn name(&self) -> &'static str {
        "shell"
    }
    fn description(&self) -> &'static str {
        "Run a shell command in the repository root (non-interactive; stdin is closed). Prefer run_test/run_lint/run_build for project commands. Dangerous, network and out-of-workspace commands require user approval."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "command":{"type":"string"},
            "timeout_secs":{"type":"integer","description":"Default 120"}
        },"required":["command"]})
    }
    fn read_only(&self) -> bool {
        false
    }
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String> {
        Ok(classify(arg_str(args, "command")?, ctx))
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Ok(cmd) = arg_str(args, "command") else {
            return ToolOutput::err("missing `command`");
        };
        let a = classify(cmd, ctx);
        if let Some(b) = a.blocked {
            return ToolOutput::err(format!("command blocked: {b}"));
        }
        match execute(cmd, ctx, timeout_arg(args, 120), &a.kinds).await {
            Ok((r, note)) => {
                let content = format_result(&r, &note, ctx, "");
                let ok = r.exit_code == Some(0);
                let mut out = ToolOutput {
                    ok,
                    content,
                    summary: format!(
                        "exit {} ({:.1}s)",
                        r.exit_code.map(|c| c.to_string()).unwrap_or_else(|| if r.timed_out { "timeout".into() } else { "?".into() }),
                        r.duration_ms as f64 / 1000.0
                    ),
                    ..Default::default()
                };
                out.data = json!({
                    "command": r.command, "exit_code": r.exit_code, "duration_ms": r.duration_ms,
                    "timed_out": r.timed_out
                });
                out
            }
            Err(e) => ToolOutput::err(format!("failed to start command: {e}")),
        }
    }
}

/// Resolve the command for a project category, optionally targeted to files.
fn project_command(ctx: &ToolContext, cat: CommandCategory, files: &[String]) -> Option<String> {
    let mut cmds = ctx.profile.all(cat);
    let first = cmds.next()?;
    if !files.is_empty() {
        if let Some(t) = &first.targeted {
            return Some(expand_targeted(t, files));
        }
    }
    if first.run.contains("{files}") {
        if files.is_empty() {
            return None;
        }
        return Some(expand_targeted(&first.run, files));
    }
    Some(first.run.clone())
}

pub struct RunTest;

impl RunTest {
    fn command(args: &Value, ctx: &ToolContext) -> Result<String, String> {
        if let Some(c) = opt_str(args, "command") {
            return Ok(c.to_string());
        }
        let files = opt_str_list(args, "files");
        // Normalise test file paths to workspace-relative.
        let files: Vec<String> = files
            .iter()
            .map(|f| {
                let (path, suffix) = split_location(f);
                let rel = ctx.workspace.resolve(path).ok().and_then(|r| r.rel).unwrap_or_else(|| path.to_string());
                format!("{rel}{suffix}")
            })
            .collect();
        project_command(ctx, CommandCategory::Test, &files).ok_or_else(|| {
            "no test command detected for this project; pass `command` (e.g. the command from the README or CI config)".to_string()
        })
    }
}

/// Split `spec/a_spec.rb:12` or `tests/t.py::test_x` into path and suffix.
fn split_location(f: &str) -> (&str, &str) {
    if let Some(i) = f.find("::") {
        return (&f[..i], &f[i..]);
    }
    if let Some((p, line)) = f.rsplit_once(':') {
        if !line.is_empty() && line.chars().all(|c| c.is_ascii_digit()) {
            return (p, &f[p.len()..]);
        }
    }
    (f, "")
}

#[async_trait::async_trait]
impl Tool for RunTest {
    fn name(&self) -> &'static str {
        "run_test"
    }
    fn description(&self) -> &'static str {
        "Run the project's tests. Pass `files` (test files, optionally with :line or ::test suffixes) to run a targeted subset, or `command` to run a specific test command. Returns exit code, pass/fail counts, failing test names and relevant output."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "files":{"type":"array","items":{"type":"string"},"description":"Test files to run (targeted run)"},
            "command":{"type":"string","description":"Explicit test command (overrides detection)"},
            "timeout_secs":{"type":"integer","description":"Default 900"}
        }})
    }
    fn read_only(&self) -> bool {
        // Tests are allowed during planning and review: they observe, and any
        // files they write are journaled.
        true
    }
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String> {
        let cmd = Self::command(args, ctx)?;
        let mut a = classify(&cmd, ctx);
        a.title = format!("test: {cmd}");
        Ok(a)
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let cmd = match Self::command(args, ctx) {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(e),
        };
        let a = classify(&cmd, ctx);
        if let Some(b) = a.blocked {
            return ToolOutput::err(format!("command blocked: {b}"));
        }
        let (r, note) = match execute(&cmd, ctx, timeout_arg(args, 900), &a.kinds).await {
            Ok(x) => x,
            Err(e) => return ToolOutput::err(format!("failed to start tests: {e}")),
        };
        let combined = format!("{}\n{}", r.stdout, r.stderr);
        let parsed = testparse::parse(&crate::output::strip_ansi(&combined));
        let report = TestReport {
            command: cmd.clone(),
            exit_code: r.exit_code,
            passed: parsed.passed,
            failed: parsed.failed,
            duration_ms: r.duration_ms,
            failed_tests: parsed.failed_tests.clone(),
        };
        let verdict = if report.succeeded() { "PASSED" } else { "FAILED" };
        let mut extra = format!("result: {verdict}");
        if let (Some(p), Some(f)) = (parsed.passed, parsed.failed) {
            extra.push_str(&format!(" ({p} passed, {f} failed)"));
        }
        extra.push('\n');
        if !parsed.failed_tests.is_empty() {
            extra.push_str(&format!("failing: {}\n", parsed.failed_tests.join(", ")));
        }
        let content = format_result(&r, &note, ctx, &extra);
        let summary = match (parsed.passed, parsed.failed) {
            (Some(p), Some(f)) => format!("{verdict}: {p} passed, {f} failed ({:.1}s)", r.duration_ms as f64 / 1000.0),
            _ => format!("{verdict} (exit {:?}, {:.1}s)", r.exit_code, r.duration_ms as f64 / 1000.0),
        };
        ToolOutput {
            ok: true, // the tool worked; test failure is reported in the content
            content,
            summary,
            data: json!({
                "command": cmd, "exit_code": r.exit_code, "duration_ms": r.duration_ms,
                "passed": parsed.passed, "failed": parsed.failed, "failed_tests": parsed.failed_tests,
            }),
            changes: vec![],
            test: Some(report),
        }
    }
}

struct CategoryTool {
    name: &'static str,
    description: &'static str,
}

async fn run_category(
    args: &Value,
    ctx: &ToolContext,
    cats: &[CommandCategory],
    label: &str,
) -> ToolOutput {
    let files = opt_str_list(args, "files");
    let cmd = match opt_str(args, "command") {
        Some(c) => Some(c.to_string()),
        None => cats.iter().find_map(|c| project_command(ctx, *c, &files)),
    };
    let Some(cmd) = cmd else {
        return ToolOutput::err(format!("no {label} command detected for this project; pass `command`"));
    };
    let a = classify(&cmd, ctx);
    if let Some(b) = a.blocked {
        return ToolOutput::err(format!("command blocked: {b}"));
    }
    match execute(&cmd, ctx, timeout_arg(args, 900), &a.kinds).await {
        Ok((r, note)) => {
            let ok = r.exit_code == Some(0);
            ToolOutput {
                ok: true,
                content: format_result(&r, &note, ctx, &format!("{label}: {}\n", if ok { "clean" } else { "problems found" })),
                summary: format!("{label} {} ({:.1}s)", if ok { "passed" } else { "failed" }, r.duration_ms as f64 / 1000.0),
                data: json!({"command": cmd, "exit_code": r.exit_code, "duration_ms": r.duration_ms}),
                ..Default::default()
            }
        }
        Err(e) => ToolOutput::err(format!("failed to start {label}: {e}")),
    }
}

fn category_assess(args: &Value, ctx: &ToolContext, cats: &[CommandCategory], label: &str) -> Result<Assessment, String> {
    let files = opt_str_list(args, "files");
    let cmd = match opt_str(args, "command") {
        Some(c) => c.to_string(),
        None => cats
            .iter()
            .find_map(|c| project_command(ctx, *c, &files))
            .ok_or_else(|| format!("no {label} command detected; pass `command`"))?,
    };
    let mut a = classify(&cmd, ctx);
    a.title = format!("{label}: {cmd}");
    Ok(a)
}

fn category_params() -> Value {
    json!({"type":"object","properties":{
        "kind":{"type":"string","enum":["lint","typecheck"],"description":"Default lint"},
        "files":{"type":"array","items":{"type":"string"}},
        "command":{"type":"string","description":"Explicit command (overrides detection)"},
        "timeout_secs":{"type":"integer"}
    }})
}

pub struct RunLint;

impl RunLint {
    const INFO: CategoryTool = CategoryTool {
        name: "run_lint",
        description: "Run the project's linter (kind=lint, default) or type checker (kind=typecheck). Falls back to the other when one is not configured.",
    };
    fn cats(args: &Value) -> Vec<CommandCategory> {
        if opt_str(args, "kind") == Some("typecheck") {
            vec![CommandCategory::Typecheck, CommandCategory::Lint]
        } else {
            vec![CommandCategory::Lint, CommandCategory::Typecheck]
        }
    }
}

#[async_trait::async_trait]
impl Tool for RunLint {
    fn name(&self) -> &'static str {
        Self::INFO.name
    }
    fn description(&self) -> &'static str {
        Self::INFO.description
    }
    fn parameters(&self) -> Value {
        category_params()
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String> {
        category_assess(args, ctx, &Self::cats(args), "lint")
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        run_category(args, ctx, &Self::cats(args), "lint").await
    }
}

pub struct RunBuild;

#[async_trait::async_trait]
impl Tool for RunBuild {
    fn name(&self) -> &'static str {
        "run_build"
    }
    fn description(&self) -> &'static str {
        "Build the project with its detected build command (or `command`)."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "command":{"type":"string"},
            "timeout_secs":{"type":"integer"}
        }})
    }
    fn read_only(&self) -> bool {
        false
    }
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String> {
        category_assess(args, ctx, &[CommandCategory::Build], "build")
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        run_category(args, ctx, &[CommandCategory::Build], "build").await
    }
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: String,
    pub line: u32,
    pub column: Option<u32>,
    pub severity: String,
    pub message: String,
}

/// Parse `path:line[:col]: message` and rustc-style `--> path:line:col`.
pub fn parse_diagnostics(output: &str) -> Vec<Diagnostic> {
    let generic = Regex::new(r"(?m)^([^\s:][^:\n]*\.[A-Za-z0-9]+):(\d+)(?::(\d+))?:?\s*(?:-\s*)?(error|warning|note|E\d+|W\d+|[A-Z]\d{3,4})?:?\s*(.*)$").expect("regex");
    let tsc = Regex::new(r"(?m)^([^\s(][^(\n]*)\((\d+),(\d+)\): (error|warning) (\w+: .*)$").expect("regex");
    let rust_head = Regex::new(r"(?m)^(error|warning)(?:\[\w+\])?: (.*)\n\s*--> ([^:\n]+):(\d+):(\d+)").expect("regex");
    let mut out = Vec::new();
    for c in rust_head.captures_iter(output) {
        out.push(Diagnostic {
            path: c[3].to_string(),
            line: c[4].parse().unwrap_or(0),
            column: c[5].parse().ok(),
            severity: c[1].to_string(),
            message: c[2].to_string(),
        });
    }
    for c in tsc.captures_iter(output) {
        out.push(Diagnostic {
            path: c[1].to_string(),
            line: c[2].parse().unwrap_or(0),
            column: c[3].parse().ok(),
            severity: c[4].to_string(),
            message: c[5].to_string(),
        });
    }
    if out.is_empty() {
        for c in generic.captures_iter(output) {
            let sev = c.get(4).map(|m| m.as_str().to_string()).unwrap_or_else(|| "error".into());
            let sev = if sev.starts_with('W') || sev == "warning" { "warning".to_string() } else if sev == "note" { sev } else { "error".to_string() };
            out.push(Diagnostic {
                path: c[1].trim_start_matches("./").to_string(),
                line: c[2].parse().unwrap_or(0),
                column: c.get(3).and_then(|m| m.as_str().parse().ok()),
                severity: sev,
                message: c[5].trim().to_string(),
            });
        }
    }
    out.dedup();
    out
}

pub struct Diagnostics;

#[async_trait::async_trait]
impl Tool for Diagnostics {
    fn name(&self) -> &'static str {
        "diagnostics"
    }
    fn description(&self) -> &'static str {
        "Collect compiler/type-checker/linter diagnostics as structured file:line entries (uses the project's typecheck or lint command). Optional `path` filters results."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string","description":"Only report diagnostics for this file or directory"},
            "command":{"type":"string"}
        }})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String> {
        category_assess(args, ctx, &[CommandCategory::Typecheck, CommandCategory::Lint], "diagnostics")
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let cmd = opt_str(args, "command").map(str::to_string).or_else(|| {
            [CommandCategory::Typecheck, CommandCategory::Lint]
                .iter()
                .find_map(|c| project_command(ctx, *c, &[]))
        });
        let Some(cmd) = cmd else {
            return ToolOutput::err("no typecheck or lint command detected for this project; pass `command`");
        };
        let a = classify(&cmd, ctx);
        if let Some(b) = a.blocked {
            return ToolOutput::err(format!("command blocked: {b}"));
        }
        let (r, _) = match execute(&cmd, ctx, timeout_arg(args, 600), &a.kinds).await {
            Ok(x) => x,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let combined = crate::output::strip_ansi(&format!("{}\n{}", r.stdout, r.stderr));
        let mut diags = parse_diagnostics(&combined);
        if let Some(filter) = opt_str(args, "path") {
            let f = filter.trim_start_matches("./");
            diags.retain(|d| d.path.starts_with(f) || d.path.ends_with(f));
        }
        let errors = diags.iter().filter(|d| d.severity == "error").count();
        let mut content = format!("$ {cmd}\nexit code {:?}; {} diagnostics ({errors} errors)\n", r.exit_code, diags.len());
        for d in diags.iter().take(100) {
            content.push_str(&format!(
                "{}:{}{}: {}: {}\n",
                d.path,
                d.line,
                d.column.map(|c| format!(":{c}")).unwrap_or_default(),
                d.severity,
                d.message
            ));
        }
        if diags.is_empty() && r.exit_code != Some(0) {
            content.push_str("(could not parse diagnostics; raw output follows)\n");
            content.push_str(&for_model(&combined, ctx.output_chars));
        }
        ToolOutput::ok(for_model(&content, ctx.output_chars), format!("{} diagnostics, {errors} errors", diags.len()))
            .with_data(serde_json::to_value(&diags).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture;

    #[test]
    fn diagnostics_parsing() {
        let rust = "error[E0308]: mismatched types\n  --> src/lib.rs:10:5\n   |\nwarning: unused variable: `x`\n --> src/main.rs:3:9\n";
        let d = parse_diagnostics(rust);
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].path.as_str(), d[0].line, d[0].severity.as_str()), ("src/lib.rs", 10, "error"));
        let tsc = "src/a.ts(3,7): error TS2322: Type 'string' is not assignable to type 'number'.\n";
        let d = parse_diagnostics(tsc);
        assert_eq!((d[0].path.as_str(), d[0].line, d[0].column), ("src/a.ts", 3, Some(7)));
        let ruff = "app/models.py:12:5: F841 Local variable `x` is assigned to but never used\n";
        let d = parse_diagnostics(ruff);
        assert_eq!((d[0].path.as_str(), d[0].line), ("app/models.py", 12));
    }

    #[test]
    fn location_splitting() {
        assert_eq!(split_location("spec/a_spec.rb:12"), ("spec/a_spec.rb", ":12"));
        assert_eq!(split_location("tests/t.py::test_x"), ("tests/t.py", "::test_x"));
        assert_eq!(split_location("tests/t.py"), ("tests/t.py", ""));
    }

    #[tokio::test]
    async fn shell_blocks_catastrophic_and_reports_structured_result() {
        let f = fixture(&[]);
        let a = Shell.assess(&json!({"command": "rm -rf /"}), &f.ctx).unwrap();
        assert!(a.blocked.is_some());
        let out = Shell.run(&json!({"command": "rm -rf /"}), &f.ctx).await;
        assert!(!out.ok && out.content.contains("blocked"));
        let out = Shell.run(&json!({"command": "echo hello"}), &f.ctx).await;
        assert!(out.ok);
        assert!(out.content.contains("hello"));
        assert_eq!(out.data["exit_code"], 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_file_changes_are_journaled_for_undo() {
        let f = fixture(&[]);
        f.ctx.journal().begin("turn", None);
        let out = Shell.run(&json!({"command": "echo generated > gen.txt"}), &f.ctx).await;
        assert!(out.ok, "{}", out.content);
        assert!(out.content.contains("gen.txt"), "{}", out.content);
        assert!(f.dir.path().join("gen.txt").exists());
        f.ctx.journal().undo(None).unwrap();
        assert!(!f.dir.path().join("gen.txt").exists());
    }

    #[tokio::test]
    async fn run_test_uses_detected_command_and_parses() {
        let f = fixture(&[
            ("pytest.ini", "[pytest]\n"),
            ("tests/test_x.py", "def test_ok():\n    assert True\n"),
        ]);
        let a = RunTest.assess(&json!({"files": ["tests/test_x.py"]}), &f.ctx).unwrap();
        assert!(a.title.contains("pytest -q tests/test_x.py"), "{}", a.title);
        assert_eq!(a.kinds, vec![ActionKind::Test]);
        let none = fixture(&[("README", "x")]);
        assert!(RunTest.assess(&json!({}), &none.ctx).is_err());
    }
}
