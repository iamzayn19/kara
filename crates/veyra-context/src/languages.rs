//! Language packs.
//!
//! A language pack is data (TOML under `languages/`): file extensions,
//! package manifests, test/lint/format/typecheck/build command discovery,
//! symbol and import patterns, and LSP/Tree-sitter hints. Users can add or
//! override packs by dropping TOML files into `~/.veyra/languages/`.
//!
//! Unknown languages are fine: they still get generic file, search, git and
//! shell capabilities.

use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

macro_rules! builtin_packs {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_str!(concat!("../../../languages/", $name, ".toml")))),*]
    };
}

/// Packs compiled into the binary.
pub const BUILTIN_PACKS: &[(&str, &str)] = builtin_packs!(
    "ruby",
    "python",
    "javascript",
    "typescript",
    "rust",
    "go",
    "java",
    "c",
    "cpp",
    "csharp",
    "php",
    "swift",
    "kotlin",
    "shell",
    "web",
    "sql",
);

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub run: String,
    /// Command restricted to specific files. Placeholders: `{files}`
    /// (shell-quoted relative paths), `{names}` (file stems), `{classes}`
    /// (file stems), `{packages}` (`./dir` per file), `{modules}` (dotted).
    pub targeted: Option<String>,
    /// All of these paths must exist (globs allowed).
    #[serde(default)]
    pub requires: Vec<String>,
    /// At least one of these paths must exist (globs allowed).
    #[serde(default)]
    pub requires_any: Vec<String>,
    /// `package.json` / `composer.json` must define this script.
    pub requires_script: Option<String>,
    /// At least one file must match this glob.
    pub requires_files_matching: Option<String>,
    pub windows_run: Option<String>,
    pub windows_targeted: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Commands {
    #[serde(default)]
    pub test: Vec<CommandSpec>,
    #[serde(default)]
    pub lint: Vec<CommandSpec>,
    #[serde(default)]
    pub typecheck: Vec<CommandSpec>,
    #[serde(default)]
    pub format: Vec<CommandSpec>,
    #[serde(default)]
    pub build: Vec<CommandSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolRule {
    pub kind: String,
    pub pattern: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Imports {
    #[serde(default)]
    pub patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackDef {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub filenames: Vec<String>,
    #[serde(default)]
    pub package_files: Vec<String>,
    #[serde(default)]
    pub test_patterns: Vec<String>,
    #[serde(default)]
    pub lsp: Vec<String>,
    /// Tree-sitter grammar name. Recorded for the planned Tree-sitter
    /// backend; v0.1 extracts symbols with the patterns below.
    pub tree_sitter: Option<String>,
    #[serde(default)]
    pub commands: Commands,
    #[serde(default)]
    pub symbols: Vec<SymbolRule>,
    #[serde(default)]
    pub imports: Imports,
}

#[derive(Debug, Clone)]
pub struct LanguagePack {
    pub def: PackDef,
    pub symbol_res: Vec<(String, Regex)>,
    pub import_res: Vec<Regex>,
    pub test_globs: globset::GlobSet,
}

impl LanguagePack {
    pub fn parse(text: &str) -> anyhow::Result<LanguagePack> {
        let def: PackDef = toml::from_str(text)?;
        let mut symbol_res = Vec::new();
        for rule in &def.symbols {
            let re = Regex::new(&format!("(?m){}", rule.pattern)).map_err(|e| {
                anyhow::anyhow!("pack {}: bad symbol pattern {}: {e}", def.id, rule.pattern)
            })?;
            symbol_res.push((rule.kind.clone(), re));
        }
        let mut import_res = Vec::new();
        for p in &def.imports.patterns {
            import_res
                .push(Regex::new(&format!("(?m){p}")).map_err(|e| {
                    anyhow::anyhow!("pack {}: bad import pattern {p}: {e}", def.id)
                })?);
        }
        let mut gb = globset::GlobSetBuilder::new();
        for p in &def.test_patterns {
            gb.add(globset::Glob::new(p)?);
        }
        Ok(LanguagePack {
            def,
            symbol_res,
            import_res,
            test_globs: gb.build()?,
        })
    }

    pub fn id(&self) -> &str {
        &self.def.id
    }

    pub fn is_test_file(&self, rel: &str) -> bool {
        if self.test_globs.is_match(rel) {
            return true;
        }
        let name = rel.rsplit('/').next().unwrap_or(rel).to_ascii_lowercase();
        name.contains("_test.")
            || name.contains("_spec.")
            || name.contains(".test.")
            || name.contains(".spec.")
            || name.starts_with("test_")
            || (name.ends_with("test.java")
                || name.ends_with("tests.cs")
                || name.ends_with("test.kt"))
    }
}

#[derive(Debug, Clone)]
pub struct LanguageRegistry {
    packs: Vec<LanguagePack>,
    by_ext: HashMap<String, usize>,
    by_name: HashMap<String, usize>,
}

impl LanguageRegistry {
    pub fn builtin() -> Self {
        let packs = BUILTIN_PACKS
            .iter()
            .map(|(name, text)| {
                LanguagePack::parse(text)
                    .unwrap_or_else(|e| panic!("builtin language pack {name} is invalid: {e}"))
            })
            .collect();
        Self::from_packs(packs)
    }

    /// Builtin packs plus user packs from `dir` (same id overrides builtin).
    /// Invalid user packs are reported, not fatal.
    pub fn with_user_dir(dir: &Path) -> (Self, Vec<String>) {
        let mut reg = Self::builtin();
        let mut warnings = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return (reg, warnings);
        };
        let mut packs = std::mem::take(&mut reg.packs);
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            match std::fs::read_to_string(&path)
                .map_err(anyhow::Error::from)
                .and_then(|t| LanguagePack::parse(&t))
            {
                Ok(pack) => {
                    packs.retain(|p| p.def.id != pack.def.id);
                    packs.push(pack);
                }
                Err(e) => warnings.push(format!("{}: {e}", path.display())),
            }
        }
        (Self::from_packs(packs), warnings)
    }

    fn from_packs(packs: Vec<LanguagePack>) -> Self {
        let mut by_ext = HashMap::new();
        let mut by_name = HashMap::new();
        for (i, p) in packs.iter().enumerate() {
            for e in &p.def.extensions {
                by_ext.entry(e.to_ascii_lowercase()).or_insert(i);
            }
            for f in &p.def.filenames {
                by_name.entry(f.clone()).or_insert(i);
            }
        }
        Self {
            packs,
            by_ext,
            by_name,
        }
    }

    pub fn packs(&self) -> &[LanguagePack] {
        &self.packs
    }

    pub fn get(&self, id: &str) -> Option<&LanguagePack> {
        self.packs.iter().find(|p| p.def.id == id)
    }

    pub fn for_path(&self, rel: &str) -> Option<&LanguagePack> {
        let name = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
        if let Some(&i) = self.by_name.get(name) {
            return Some(&self.packs[i]);
        }
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        self.by_ext.get(&ext).map(|&i| &self.packs[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_packs_parse() {
        let reg = LanguageRegistry::builtin();
        assert_eq!(reg.packs().len(), 16);
        for id in [
            "ruby",
            "python",
            "javascript",
            "typescript",
            "rust",
            "go",
            "java",
            "c",
            "cpp",
            "csharp",
            "php",
            "swift",
            "kotlin",
            "shell",
            "web",
            "sql",
        ] {
            assert!(reg.get(id).is_some(), "{id}");
        }
    }

    #[test]
    fn lookup_by_extension_and_name() {
        let reg = LanguageRegistry::builtin();
        assert_eq!(reg.for_path("app/models/user.rb").unwrap().id(), "ruby");
        assert_eq!(reg.for_path("Gemfile").unwrap().id(), "ruby");
        assert_eq!(reg.for_path("src/lib.rs").unwrap().id(), "rust");
        assert_eq!(reg.for_path("web/App.TSX").unwrap().id(), "typescript");
        assert_eq!(reg.for_path("db/schema.sql").unwrap().id(), "sql");
        assert!(reg.for_path("notes.unknownext").is_none());
        assert!(reg.for_path("LICENSE").is_none());
    }

    #[test]
    fn test_file_detection() {
        let reg = LanguageRegistry::builtin();
        assert!(reg
            .get("ruby")
            .unwrap()
            .is_test_file("spec/models/user_spec.rb"));
        assert!(reg
            .get("python")
            .unwrap()
            .is_test_file("tests/test_auth.py"));
        assert!(reg
            .get("go")
            .unwrap()
            .is_test_file("pkg/auth/session_test.go"));
        assert!(reg
            .get("typescript")
            .unwrap()
            .is_test_file("src/auth.test.ts"));
        assert!(!reg.get("python").unwrap().is_test_file("app/auth.py"));
    }

    #[test]
    fn user_pack_overrides_and_bad_pack_warns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("zig.toml"),
            "id = \"zig\"\nname = \"Zig\"\nextensions = [\"zig\"]\n[[commands.test]]\nrun = \"zig build test\"\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("broken.toml"), "id = 3").unwrap();
        let (reg, warnings) = LanguageRegistry::with_user_dir(dir.path());
        assert_eq!(reg.for_path("main.zig").unwrap().id(), "zig");
        assert_eq!(warnings.len(), 1);
    }
}
