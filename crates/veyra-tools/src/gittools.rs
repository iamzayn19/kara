//! Read-only git tools. Arguments are passed to git directly (no shell), and
//! revisions are validated so model input cannot inject git options.

use crate::output::for_model;
use crate::{arg_str, opt_bool, opt_str, opt_u64, Assessment, Tool, ToolContext, ToolOutput};
use serde_json::{json, Value};
use veyra_context::git;
use veyra_protocol::ActionKind;

fn valid_rev(rev: &str) -> Result<&str, String> {
    let r = rev.trim();
    if r.is_empty() || r.starts_with('-') || r.contains(['\0', '\n', ' ']) || r.len() > 200 {
        return Err(format!("invalid revision `{rev}`"));
    }
    Ok(r)
}

fn rel_path(ctx: &ToolContext, p: &str) -> Result<String, String> {
    let r = ctx.workspace.resolve(p).map_err(|e| e.to_string())?;
    r.rel.ok_or_else(|| format!("{p} is outside the workspace"))
}

fn run_git(ctx: &ToolContext, args: &[&str]) -> ToolOutput {
    if !git::is_repo(ctx.root()) {
        return ToolOutput::err("not a git repository");
    }
    match git::git(ctx.root(), args) {
        Ok(out) => {
            let lines = out.lines().count();
            let text = if out.trim().is_empty() { "(no output)".to_string() } else { out };
            ToolOutput::ok(for_model(&text, ctx.output_chars), format!("{lines} lines"))
        }
        Err(e) => ToolOutput::err(e.to_string()),
    }
}

fn git_assess(title: String) -> Result<Assessment, String> {
    Ok(Assessment::new(title, vec![ActionKind::GitRead]))
}

pub struct GitStatus;

#[async_trait::async_trait]
impl Tool for GitStatus {
    fn name(&self) -> &'static str {
        "git_status"
    }
    fn description(&self) -> &'static str {
        "Show branch and changed files, distinguishing changes made by Veyra in this session from the user's own uncommitted changes."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, _: &Value, _: &ToolContext) -> Result<Assessment, String> {
        git_assess("git status".into())
    }
    async fn run(&self, _: &Value, ctx: &ToolContext) -> ToolOutput {
        if !git::is_repo(ctx.root()) {
            return ToolOutput::err("not a git repository");
        }
        let entries = match git::status(ctx.root()) {
            Ok(e) => e,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let (veyra, baseline) = {
            let j = ctx.journal();
            (j.veyra_changed_paths(), j.baseline_dirty().clone())
        };
        let branch = git::branch(ctx.root()).unwrap_or_else(|| "(detached)".into());
        let mut content = format!("branch: {branch}\n");
        let mut rows = Vec::new();
        for e in &entries {
            let owner = match (veyra.contains(&e.path), baseline.contains(&e.path)) {
                (true, true) => "user+veyra",
                (true, false) => "veyra",
                _ => "user",
            };
            content.push_str(&format!("{} {} [{owner}]\n", e.code, e.path));
            rows.push(json!({"path": e.path, "code": e.code, "owner": owner}));
        }
        if entries.is_empty() {
            content.push_str("working tree clean\n");
        }
        ToolOutput::ok(content, format!("{} changed files", entries.len()))
            .with_data(json!({"branch": branch, "files": rows}))
    }
}

pub struct GitDiff;

#[async_trait::async_trait]
impl Tool for GitDiff {
    fn name(&self) -> &'static str {
        "git_diff"
    }
    fn description(&self) -> &'static str {
        "Show uncommitted changes (`git diff`). Options: `path`, `staged`, `base` (compare against a revision), `stat` (summary only)."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "staged":{"type":"boolean"},
            "base":{"type":"string","description":"Revision to diff against, e.g. HEAD~1 or main"},
            "stat":{"type":"boolean"}
        }})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, _: &Value, _: &ToolContext) -> Result<Assessment, String> {
        git_assess("git diff".into())
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let mut a: Vec<String> = vec!["diff".into(), "--no-ext-diff".into()];
        if opt_bool(args, "staged").unwrap_or(false) {
            a.push("--cached".into());
        }
        if opt_bool(args, "stat").unwrap_or(false) {
            a.push("--stat".into());
        }
        if let Some(b) = opt_str(args, "base") {
            match valid_rev(b) {
                Ok(b) => a.push(b.to_string()),
                Err(e) => return ToolOutput::err(e),
            }
        }
        if let Some(p) = opt_str(args, "path") {
            match rel_path(ctx, p) {
                Ok(r) => {
                    a.push("--".into());
                    a.push(r);
                }
                Err(e) => return ToolOutput::err(e),
            }
        }
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        let mut out = run_git(ctx, &refs);
        // Untracked files do not appear in `git diff`; list them.
        if out.ok && opt_str(args, "base").is_none() && !opt_bool(args, "staged").unwrap_or(false) {
            if let Ok(entries) = git::status(ctx.root()) {
                let untracked: Vec<&str> = entries.iter().filter(|e| e.untracked()).map(|e| e.path.as_str()).collect();
                if !untracked.is_empty() {
                    out.content.push_str(&format!("\nuntracked files (not shown above): {}\n", untracked.join(", ")));
                }
            }
        }
        out
    }
}

pub struct GitLog;

#[async_trait::async_trait]
impl Tool for GitLog {
    fn name(&self) -> &'static str {
        "git_log"
    }
    fn description(&self) -> &'static str {
        "Show recent commits (one line each), optionally for a path."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "limit":{"type":"integer","description":"Default 20"}
        }})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, _: &Value, _: &ToolContext) -> Result<Assessment, String> {
        git_assess("git log".into())
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let n = format!("-n{}", opt_u64(args, "limit").unwrap_or(20).clamp(1, 500));
        let mut a = vec!["log".to_string(), n, "--date=short".into(), "--pretty=format:%h %ad %an %s".into()];
        if let Some(p) = opt_str(args, "path") {
            match rel_path(ctx, p) {
                Ok(r) => {
                    a.push("--".into());
                    a.push(r);
                }
                Err(e) => return ToolOutput::err(e),
            }
        }
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        run_git(ctx, &refs)
    }
}

pub struct GitShow;

#[async_trait::async_trait]
impl Tool for GitShow {
    fn name(&self) -> &'static str {
        "git_show"
    }
    fn description(&self) -> &'static str {
        "Show a commit (message and diff), or a file at a revision when `path` is given."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "rev":{"type":"string"},
            "path":{"type":"string"}
        },"required":["rev"]})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, args: &Value, _: &ToolContext) -> Result<Assessment, String> {
        git_assess(format!("git show {}", arg_str(args, "rev")?))
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let rev = match arg_str(args, "rev").and_then(valid_rev) {
            Ok(r) => r.to_string(),
            Err(e) => return ToolOutput::err(e),
        };
        match opt_str(args, "path") {
            Some(p) => match rel_path(ctx, p) {
                Ok(r) => run_git(ctx, &["show", &format!("{rev}:{r}")]),
                Err(e) => ToolOutput::err(e),
            },
            None => run_git(ctx, &["show", "--stat", "--patch", "--no-ext-diff", &rev]),
        }
    }
}

pub struct GitBlame;

#[async_trait::async_trait]
impl Tool for GitBlame {
    fn name(&self) -> &'static str {
        "git_blame"
    }
    fn description(&self) -> &'static str {
        "Show who last changed each line of a file (optionally a line range)."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "path":{"type":"string"},
            "start_line":{"type":"integer"},
            "end_line":{"type":"integer"}
        },"required":["path"]})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, args: &Value, _: &ToolContext) -> Result<Assessment, String> {
        git_assess(format!("git blame {}", arg_str(args, "path")?))
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let rel = match arg_str(args, "path").and_then(|p| rel_path(ctx, p)) {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(e),
        };
        let mut a = vec!["blame".to_string(), "--date=short".into()];
        if let Some(s) = opt_u64(args, "start_line") {
            let e = opt_u64(args, "end_line").unwrap_or(s + 40).max(s);
            a.push(format!("-L{s},{e}"));
        }
        a.push("--".into());
        a.push(rel);
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        run_git(ctx, &refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture;
    use std::process::Command;

    fn init_repo(dir: &std::path::Path) {
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "t"],
            vec!["add", "."],
            vec!["commit", "-qm", "init"],
        ] {
            assert!(Command::new("git").arg("-C").arg(dir).args(&args).status().unwrap().success());
        }
    }

    #[test]
    fn rev_validation() {
        assert!(valid_rev("HEAD~1").is_ok());
        assert!(valid_rev("--output=/tmp/x").is_err());
        assert!(valid_rev("a b").is_err());
    }

    #[tokio::test]
    async fn status_distinguishes_owners() {
        let f = fixture(&[("a.txt", "a\n"), ("b.txt", "b\n")]);
        init_repo(f.dir.path());
        std::fs::write(f.dir.path().join("a.txt"), "user change\n").unwrap();
        f.ctx.journal().set_baseline_dirty(["a.txt".to_string()]);
        let out = crate::fs::ReadFile.run(&json!({"path": "b.txt"}), &f.ctx).await;
        assert!(out.ok);
        let out = crate::fs::EditFile.run(&json!({"path": "b.txt", "old_string": "b", "new_string": "B"}), &f.ctx).await;
        assert!(out.ok, "{}", out.content);
        let out = GitStatus.run(&json!({}), &f.ctx).await;
        assert!(out.content.contains("a.txt [user]"), "{}", out.content);
        assert!(out.content.contains("b.txt [veyra]"), "{}", out.content);
        let out = GitDiff.run(&json!({"path": "b.txt"}), &f.ctx).await;
        assert!(out.content.contains("+B"));
        let out = GitShow.run(&json!({"rev": "--help"}), &f.ctx).await;
        assert!(!out.ok);
        let out = GitLog.run(&json!({}), &f.ctx).await;
        assert!(out.content.contains("init"));
    }
}
