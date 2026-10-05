//! Project profile: which languages are present and which commands run the
//! project's tests, linters, type checkers, formatters and builds.

use crate::languages::{CommandSpec, LanguageRegistry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandCategory {
    Test,
    Lint,
    Typecheck,
    Format,
    Build,
}

impl CommandCategory {
    pub fn label(self) -> &'static str {
        match self {
            CommandCategory::Test => "test",
            CommandCategory::Lint => "lint",
            CommandCategory::Typecheck => "typecheck",
            CommandCategory::Format => "format",
            CommandCategory::Build => "build",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCommand {
    pub category: CommandCategory,
    pub language: String,
    pub run: String,
    pub targeted: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectProfile {
    /// Language id -> file count (from a bounded scan).
    pub languages: BTreeMap<String, usize>,
    /// Languages whose package manifest was found at the root.
    pub manifests: Vec<String>,
    pub commands: Vec<ProjectCommand>,
    /// Node package manager when package.json exists.
    pub package_manager: Option<String>,
}

const SCAN_LIMIT: usize = 20_000;

impl ProjectProfile {
    pub fn detect(root: &Path, registry: &LanguageRegistry) -> ProjectProfile {
        let mut profile = ProjectProfile::default();

        // Bounded scan for language mix (respects .gitignore).
        let walker = ignore::WalkBuilder::new(root)
            .hidden(true)
            .git_ignore(true)
            .max_depth(Some(12))
            .build();
        for entry in walker.flatten().take(SCAN_LIMIT) {
            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
                if let Some(pack) = registry.for_path(&rel.to_string_lossy()) {
                    *profile.languages.entry(pack.id().to_string()).or_default() += 1;
                }
            }
        }

        let package_json = read_scripts(&root.join("package.json"));
        let composer_json = read_scripts(&root.join("composer.json"));
        if root.join("package.json").exists() {
            profile.package_manager = Some(detect_pm(root).to_string());
        }

        for pack in registry.packs() {
            if pack.def.package_files.iter().any(|p| exists_glob(root, p)) {
                profile.manifests.push(pack.id().to_string());
            }
        }

        // Order languages: manifests first, then by file count.
        let mut order: Vec<String> = profile.languages.keys().cloned().collect();
        for m in &profile.manifests {
            if !order.contains(m) {
                order.push(m.clone());
            }
        }
        order.sort_by_key(|id| {
            let manifest = profile.manifests.contains(id);
            let count = profile.languages.get(id).copied().unwrap_or(0);
            (!manifest, std::cmp::Reverse(count))
        });

        let scripts_for = |lang: &str| -> &[String] {
            if lang == "php" {
                &composer_json
            } else {
                &package_json
            }
        };

        for lang in &order {
            let Some(pack) = registry.get(lang) else {
                continue;
            };
            let cats: [(CommandCategory, &Vec<CommandSpec>); 5] = [
                (CommandCategory::Test, &pack.def.commands.test),
                (CommandCategory::Lint, &pack.def.commands.lint),
                (CommandCategory::Typecheck, &pack.def.commands.typecheck),
                (CommandCategory::Format, &pack.def.commands.format),
                (CommandCategory::Build, &pack.def.commands.build),
            ];
            for (cat, specs) in cats {
                if let Some(spec) = specs.iter().find(|s| satisfied(root, s, scripts_for(lang))) {
                    let pm = profile.package_manager.as_deref().unwrap_or("npm");
                    let (run, targeted) = if cfg!(windows) {
                        (
                            spec.windows_run.clone().unwrap_or_else(|| spec.run.clone()),
                            spec.windows_targeted
                                .clone()
                                .or_else(|| spec.targeted.clone()),
                        )
                    } else {
                        (spec.run.clone(), spec.targeted.clone())
                    };
                    let cmd = ProjectCommand {
                        category: cat,
                        language: lang.clone(),
                        run: run.replace("{pm}", pm),
                        targeted: targeted.map(|t| t.replace("{pm}", pm)),
                    };
                    // Same command from two packs (JS + TS) only once.
                    if !profile
                        .commands
                        .iter()
                        .any(|c| c.category == cat && c.run == cmd.run)
                    {
                        profile.commands.push(cmd);
                    }
                }
            }
        }
        profile
    }

    pub fn first(&self, cat: CommandCategory) -> Option<&ProjectCommand> {
        self.commands
            .iter()
            .find(|c| c.category == cat && !c.run.contains("{files}"))
    }

    pub fn all(&self, cat: CommandCategory) -> impl Iterator<Item = &ProjectCommand> {
        self.commands.iter().filter(move |c| c.category == cat)
    }

    /// `(command, permission category)` pairs for the command classifier.
    pub fn known_commands(&self) -> Vec<(String, CommandCategory)> {
        let mut out = Vec::new();
        for c in &self.commands {
            let base = c.run.split("{").next().unwrap_or(&c.run).trim().to_string();
            if !base.is_empty() {
                out.push((base, c.category));
            }
            if let Some(t) = &c.targeted {
                let base = t.split('{').next().unwrap_or(t).trim().to_string();
                if !base.is_empty() {
                    out.push((base, c.category));
                }
            }
        }
        out
    }

    pub fn primary_language(&self) -> Option<&str> {
        self.languages
            .iter()
            .max_by_key(|(id, n)| (self.manifests.contains(id), **n))
            .map(|(id, _)| id.as_str())
    }
}

/// Expand a targeted command template for specific files.
pub fn expand_targeted(template: &str, files: &[String]) -> String {
    let quote = |s: &str| quote_arg(s);
    let stem = |f: &str| {
        let name = f.rsplit('/').next().unwrap_or(f);
        name.split('.').next().unwrap_or(name).to_string()
    };
    let files_q: Vec<String> = files.iter().map(|f| quote(f)).collect();
    let names: Vec<String> = files.iter().map(|f| quote(&stem(f))).collect();
    let mut packages: Vec<String> = files
        .iter()
        .map(|f| match f.rsplit_once('/') {
            Some((dir, _)) => quote(&format!("./{dir}")),
            None => "./".to_string() + ".",
        })
        .collect();
    packages.dedup();
    let modules: Vec<String> = files
        .iter()
        .map(|f| quote(&f.trim_end_matches(".py").replace('/', ".")))
        .collect();
    template
        .replace("{files}", &files_q.join(" "))
        .replace("{names}", &names.join(" "))
        .replace("{classes}", &names.join(","))
        .replace("{packages}", &packages.join(" "))
        .replace("{modules}", &modules.join(" "))
}

/// Quote one argument for the platform shell Veyra runs commands with
/// (`sh -c` on Unix, `cmd /C` on Windows).
pub fn quote_arg(s: &str) -> String {
    if cfg!(windows) {
        let safe = !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_./\\:=,+@".contains(c));
        if safe {
            s.to_string()
        } else {
            // Inside double quotes cmd treats & | < > ^ literally; % and " are
            // neutralised.
            format!("\"{}\"", s.replace('"', "").replace('%', "%%"))
        }
    } else {
        shell_words::quote(s).into_owned()
    }
}

fn satisfied(root: &Path, spec: &CommandSpec, scripts: &[String]) -> bool {
    if !spec.requires.iter().all(|p| exists_glob(root, p)) {
        return false;
    }
    if !spec.requires_any.is_empty() && !spec.requires_any.iter().any(|p| exists_glob(root, p)) {
        return false;
    }
    if let Some(script) = &spec.requires_script {
        if !scripts.iter().any(|s| s == script) {
            return false;
        }
    }
    if let Some(glob) = &spec.requires_files_matching {
        if !exists_glob(root, glob) {
            return false;
        }
    }
    true
}

fn exists_glob(root: &Path, pattern: &str) -> bool {
    if !pattern.contains(['*', '?', '[']) {
        return root.join(pattern).exists();
    }
    let Ok(glob) = globset::Glob::new(pattern) else {
        return false;
    };
    let m = glob.compile_matcher();
    let depth = pattern.matches('/').count() + 1;
    ignore::WalkBuilder::new(root)
        .max_depth(Some(depth))
        .hidden(false)
        .build()
        .flatten()
        .any(|e| {
            e.path()
                .strip_prefix(root)
                .map(|r| m.is_match(r))
                .unwrap_or(false)
        })
}

fn read_scripts(path: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    v.get("scripts")
        .and_then(|s| s.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

fn detect_pm(root: &Path) -> &'static str {
    if root.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if root.join("yarn.lock").exists() {
        "yarn"
    } else if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
        "bun"
    } else {
        "npm"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn ruby_rspec_project() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "Gemfile", "gem 'rspec'\n");
        write(d.path(), "lib/auth.rb", "class Auth; end\n");
        write(d.path(), "spec/auth_spec.rb", "describe Auth do; end\n");
        write(d.path(), ".rubocop.yml", "");
        let p = ProjectProfile::detect(d.path(), &LanguageRegistry::builtin());
        assert_eq!(
            p.first(CommandCategory::Test).unwrap().run,
            "bundle exec rspec"
        );
        assert_eq!(
            p.first(CommandCategory::Lint).unwrap().run,
            "bundle exec rubocop"
        );
        assert_eq!(p.primary_language(), Some("ruby"));
    }

    #[test]
    fn node_project_uses_lockfile_package_manager() {
        let d = tempfile::tempdir().unwrap();
        write(
            d.path(),
            "package.json",
            r#"{"scripts": {"test": "vitest", "build": "tsc", "lint": "eslint ."}}"#,
        );
        write(d.path(), "pnpm-lock.yaml", "");
        write(d.path(), "tsconfig.json", "{}");
        write(d.path(), "src/index.ts", "export const a = 1;\n");
        let p = ProjectProfile::detect(d.path(), &LanguageRegistry::builtin());
        assert_eq!(p.first(CommandCategory::Test).unwrap().run, "pnpm test");
        assert_eq!(
            p.first(CommandCategory::Build).unwrap().run,
            "pnpm run build"
        );
        assert_eq!(
            p.first(CommandCategory::Typecheck).unwrap().run,
            "npx --no-install tsc --noEmit"
        );
        assert_eq!(
            p.all(CommandCategory::Test).count(),
            1,
            "no JS/TS duplicate"
        );
    }

    #[test]
    fn rust_and_unknown() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "Cargo.toml", "[package]\nname='x'\n");
        write(d.path(), "src/lib.rs", "pub fn a() {}\n");
        let p = ProjectProfile::detect(d.path(), &LanguageRegistry::builtin());
        assert_eq!(p.first(CommandCategory::Test).unwrap().run, "cargo test");

        let d = tempfile::tempdir().unwrap();
        write(d.path(), "notes.txt", "hello");
        let p = ProjectProfile::detect(d.path(), &LanguageRegistry::builtin());
        assert!(p.commands.is_empty());
        assert!(p.languages.is_empty());
    }

    #[test]
    fn targeted_expansion_quotes_hostile_names() {
        let cmd = expand_targeted(
            "python3 -m pytest -q {files}",
            &["tests/test_a.py".into(), "tests/x; rm -rf ~.py".into()],
        );
        if cfg!(windows) {
            assert_eq!(
                cmd,
                "python3 -m pytest -q tests/test_a.py \"tests/x; rm -rf ~.py\""
            );
        } else {
            assert_eq!(
                cmd,
                "python3 -m pytest -q tests/test_a.py 'tests/x; rm -rf ~.py'"
            );
        }
        assert_eq!(
            expand_targeted("go test {packages}", &["pkg/auth/a_test.go".into()]),
            "go test ./pkg/auth"
        );
        assert_eq!(
            expand_targeted("cargo test {names}", &["tests/login.rs".into()]),
            "cargo test login"
        );
    }
}
