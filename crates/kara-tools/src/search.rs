//! Search tools: find files, grep, symbol definitions and references.

use crate::output::for_model;
use crate::{arg_str, opt_bool, opt_str, opt_u64, Assessment, Tool, ToolContext, ToolOutput};
use kara_context::search::{find_files, grep, GrepOptions};
use kara_protocol::ActionKind;
use kara_sandbox::injection;
use serde_json::{json, Value};

fn search_assess(
    title: String,
    path: Option<&str>,
    ctx: &ToolContext,
) -> Result<Assessment, String> {
    let mut a = Assessment::new(title, vec![ActionKind::Search]);
    if let Some(p) = path {
        let r = ctx.workspace.resolve(p).map_err(|e| e.to_string())?;
        crate::path_kinds(ctx, &r, false, &mut a);
    }
    Ok(a)
}

pub struct FindFiles;

#[async_trait::async_trait]
impl Tool for FindFiles {
    fn name(&self) -> &'static str {
        "find_files"
    }
    fn description(&self) -> &'static str {
        "Find files by glob (e.g. `**/*_spec.rb`, `src/**/*.ts`) or by substring of the path (e.g. `auth`)."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "pattern":{"type":"string"},
            "limit":{"type":"integer","description":"Maximum results (default 100)"}
        },"required":["pattern"]})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String> {
        search_assess(
            format!("find files `{}`", arg_str(args, "pattern")?),
            None,
            ctx,
        )
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Ok(pattern) = arg_str(args, "pattern") else {
            return ToolOutput::err("missing `pattern`");
        };
        let limit = opt_u64(args, "limit").unwrap_or(100).clamp(1, 1000) as usize;
        let root = ctx.root().to_path_buf();
        let pat = pattern.to_string();
        let (files, truncated) =
            tokio::task::spawn_blocking(move || find_files(&root, &pat, limit))
                .await
                .unwrap_or_default();
        let mut content = files.join("\n");
        if files.is_empty() {
            content = format!("no files match `{pattern}`");
        }
        if truncated {
            content.push_str(&format!(
                "\n… more than {limit} matches; refine the pattern"
            ));
        }
        ToolOutput::ok(content, format!("{} files", files.len()))
            .with_data(json!({"files": files, "truncated": truncated}))
    }
}

pub struct Grep;

#[async_trait::async_trait]
impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "Search file contents (respects .gitignore). `pattern` is a regular expression unless literal=true. Optional `path` (directory) and `glob` (e.g. `*.py`) narrow the search. Returns path:line: text."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "pattern":{"type":"string"},
            "path":{"type":"string","description":"Directory to search (default: repository root)"},
            "glob":{"type":"string","description":"File glob filter, e.g. *.rb"},
            "literal":{"type":"boolean","description":"Treat pattern as plain text"},
            "case_insensitive":{"type":"boolean"},
            "whole_word":{"type":"boolean"},
            "max_results":{"type":"integer"}
        },"required":["pattern"]})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String> {
        search_assess(
            format!("grep `{}`", arg_str(args, "pattern")?),
            opt_str(args, "path"),
            ctx,
        )
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Ok(pattern) = arg_str(args, "pattern") else {
            return ToolOutput::err("missing `pattern`");
        };
        let path = opt_str(args, "path").map(|p| {
            ctx.workspace
                .resolve(p)
                .ok()
                .and_then(|r| r.rel)
                .unwrap_or_else(|| p.to_string())
        });
        let mut opts = GrepOptions {
            regex: !opt_bool(args, "literal").unwrap_or(false),
            case_insensitive: opt_bool(args, "case_insensitive").unwrap_or(false),
            whole_word: opt_bool(args, "whole_word").unwrap_or(false),
            glob: opt_str(args, "glob").map(str::to_string),
            path,
            max_matches: opt_u64(args, "max_results").unwrap_or(100).clamp(1, 1000) as usize,
            ..Default::default()
        };
        let root = ctx.root().to_path_buf();
        let pat = pattern.to_string();
        let mut note = String::new();
        let o = opts.clone();
        let mut result = tokio::task::spawn_blocking(move || grep(&root, &pat, &o))
            .await
            .unwrap();
        if result.is_err() && opts.regex {
            // Models often pass code with unescaped parentheses; retry literally.
            opts.regex = false;
            note = "(pattern was not a valid regex; searched literally)\n".into();
            let root = ctx.root().to_path_buf();
            let pat = pattern.to_string();
            let o = opts.clone();
            result = tokio::task::spawn_blocking(move || grep(&root, &pat, &o))
                .await
                .unwrap();
        }
        let r = match result {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(format!("invalid pattern: {e}")),
        };
        let mut content = note;
        for m in &r.matches {
            content.push_str(&format!("{}:{}: {}\n", m.path, m.line, m.text));
        }
        if r.matches.is_empty() {
            content.push_str(&format!(
                "no matches for `{pattern}` in {} files",
                r.files_searched
            ));
        }
        if r.truncated {
            content.push_str("… results truncated; narrow with path/glob\n");
        }
        let labels = injection::scan(&content);
        let mut content = for_model(&content, ctx.output_chars);
        if !labels.is_empty() {
            content.push('\n');
            content.push_str(&injection::warning(&labels));
        }
        ToolOutput::ok(
            content,
            format!("{} matches in {} files", r.matches.len(), r.files_with_matches),
        )
        .with_data(json!({"matches": r.matches.len(), "files": r.files_with_matches, "truncated": r.truncated}))
    }
}

pub struct FindSymbol;

#[async_trait::async_trait]
impl Tool for FindSymbol {
    fn name(&self) -> &'static str {
        "find_symbol"
    }
    fn description(&self) -> &'static str {
        "Find where a class, function, method, type or constant is defined, using Kara's repository index. Exact matches first, then prefix and substring matches."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "name":{"type":"string"},
            "limit":{"type":"integer"}
        },"required":["name"]})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String> {
        search_assess(
            format!("find symbol `{}`", arg_str(args, "name")?),
            None,
            ctx,
        )
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Ok(name) = arg_str(args, "name") else {
            return ToolOutput::err("missing `name`");
        };
        let limit = opt_u64(args, "limit").unwrap_or(30).clamp(1, 200) as usize;
        let Some(index) = ctx.index.clone() else {
            return ToolOutput::err("repository index unavailable; use grep");
        };
        let n = name.to_string();
        let hits = tokio::task::spawn_blocking(move || {
            let _ = index.refresh();
            index.find_symbol(&n, limit)
        })
        .await
        .unwrap();
        match hits {
            Ok(hits) if hits.is_empty() => ToolOutput::ok(
                format!("no definition named like `{name}` in the index; try grep"),
                "0 definitions",
            ),
            Ok(hits) => {
                let content: String = hits
                    .iter()
                    .map(|h| format!("{}:{}: {} {}\n", h.path, h.line, h.kind, h.name))
                    .collect();
                ToolOutput::ok(content, format!("{} definitions", hits.len()))
                    .with_data(serde_json::to_value(&hits).unwrap_or_default())
            }
            Err(e) => ToolOutput::err(format!("index error: {e}")),
        }
    }
}

pub struct FindReferences;

#[async_trait::async_trait]
impl Tool for FindReferences {
    fn name(&self) -> &'static str {
        "find_references"
    }
    fn description(&self) -> &'static str {
        "Find whole-word references to an identifier across the repository, marking definitions."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object","properties":{
            "name":{"type":"string"},
            "path":{"type":"string","description":"Optional directory to restrict the search"},
            "glob":{"type":"string"}
        },"required":["name"]})
    }
    fn read_only(&self) -> bool {
        true
    }
    fn assess(&self, args: &Value, ctx: &ToolContext) -> Result<Assessment, String> {
        search_assess(
            format!("references to `{}`", arg_str(args, "name")?),
            opt_str(args, "path"),
            ctx,
        )
    }
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Ok(name) = arg_str(args, "name") else {
            return ToolOutput::err("missing `name`");
        };
        let opts = GrepOptions {
            regex: false,
            whole_word: true,
            glob: opt_str(args, "glob").map(str::to_string),
            path: opt_str(args, "path").map(str::to_string),
            max_matches: 200,
            ..Default::default()
        };
        let root = ctx.root().to_path_buf();
        let n = name.to_string();
        let r = match tokio::task::spawn_blocking(move || grep(&root, &n, &opts))
            .await
            .unwrap()
        {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let defs: Vec<(String, u32)> = ctx
            .index
            .as_ref()
            .and_then(|i| i.find_symbol(name, 50).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|h| h.name == name)
            .map(|h| (h.path, h.line))
            .collect();
        let mut content = String::new();
        for m in &r.matches {
            let tag = if defs.contains(&(m.path.clone(), m.line)) {
                " [definition]"
            } else {
                ""
            };
            content.push_str(&format!(
                "{}:{}:{} {}\n",
                m.path,
                m.line,
                tag,
                m.text.trim()
            ));
        }
        if r.matches.is_empty() {
            content = format!("no references to `{name}`");
        }
        if r.truncated {
            content.push_str("… truncated\n");
        }
        ToolOutput::ok(
            for_model(&content, ctx.output_chars),
            format!(
                "{} references in {} files",
                r.matches.len(),
                r.files_with_matches
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture;
    use std::sync::Arc;

    #[tokio::test]
    async fn grep_falls_back_to_literal() {
        let f = fixture(&[("a.py", "total(items)\n")]);
        let out = Grep.run(&json!({"pattern": "total(items"}), &f.ctx).await;
        assert!(out.ok, "{}", out.content);
        assert!(out.content.contains("searched literally"));
        assert!(out.content.contains("a.py:1:"));
    }

    #[tokio::test]
    async fn symbols_and_references() {
        let mut f = fixture(&[
            ("lib/cart.rb", "class Cart\n  def total\n  end\nend\n"),
            ("lib/order.rb", "Cart.new.total\ncart_total = 1\n"),
        ]);
        let cache = tempfile::tempdir().unwrap();
        let idx = kara_context::RepoIndex::open(
            f.dir.path(),
            cache.path(),
            kara_context::LanguageRegistry::builtin(),
            1_000_000,
        )
        .unwrap();
        idx.refresh().unwrap();
        f.ctx.index = Some(Arc::new(idx));
        let out = FindSymbol.run(&json!({"name": "total"}), &f.ctx).await;
        assert!(
            out.content.contains("lib/cart.rb:2: method total"),
            "{}",
            out.content
        );
        let out = FindReferences.run(&json!({"name": "total"}), &f.ctx).await;
        assert!(
            out.content.contains("lib/cart.rb:2: [definition]"),
            "{}",
            out.content
        );
        assert!(out.content.contains("lib/order.rb:1:"));
        assert!(!out.content.contains("lib/order.rb:2:"), "whole word only");
    }
}
