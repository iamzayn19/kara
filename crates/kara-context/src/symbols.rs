//! Symbol and import extraction.
//!
//! v0.1 uses the pattern rules from each language pack. The extractor is
//! behind a trait so a Tree-sitter backend can replace it per language
//! without touching the index or the agent.

use crate::languages::LanguagePack;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    /// 1-based line number.
    pub line: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Extracted {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<String>,
    pub lines: u32,
}

pub trait SymbolExtractor: Send + Sync {
    fn extract(&self, pack: &LanguagePack, text: &str) -> Extracted;
}

/// Regex-based extraction driven by language pack data.
#[derive(Debug, Default, Clone, Copy)]
pub struct PatternExtractor;

impl SymbolExtractor for PatternExtractor {
    fn extract(&self, pack: &LanguagePack, text: &str) -> Extracted {
        let mut out = Extracted {
            lines: text.lines().count() as u32,
            ..Default::default()
        };
        // Precompute line starts for offset -> line mapping.
        let mut starts = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        let line_of = |off: usize| match starts.binary_search(&off) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        };
        let mut seen = std::collections::HashSet::new();
        for (kind, re) in &pack.symbol_res {
            for caps in re.captures_iter(text) {
                // Last participating group is the name (patterns may have
                // optional groups before it).
                let m = (1..caps.len()).rev().find_map(|g| caps.get(g));
                let Some(m) = m else { continue };
                let name = m.as_str().trim_matches('"').to_string();
                if name.is_empty() || is_keyword(&name) {
                    continue;
                }
                let line = line_of(m.start());
                if seen.insert((name.clone(), line)) {
                    out.symbols.push(Symbol {
                        name,
                        kind: kind.clone(),
                        line,
                    });
                }
            }
        }
        out.symbols.sort_by_key(|s| s.line);
        for re in &pack.import_res {
            for caps in re.captures_iter(text) {
                if let Some(m) = caps.get(1) {
                    let t = m.as_str().to_string();
                    if !out.imports.contains(&t) {
                        out.imports.push(t);
                    }
                }
            }
        }
        out
    }
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "for"
            | "while"
            | "switch"
            | "catch"
            | "return"
            | "else"
            | "new"
            | "function"
            | "do"
            | "try"
            | "match"
            | "loop"
            | "sizeof"
            | "defined"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::LanguageRegistry;

    fn extract(lang: &str, text: &str) -> Extracted {
        let reg = LanguageRegistry::builtin();
        PatternExtractor.extract(reg.get(lang).unwrap(), text)
    }

    fn names(e: &Extracted) -> Vec<(&str, &str)> {
        e.symbols
            .iter()
            .map(|s| (s.name.as_str(), s.kind.as_str()))
            .collect()
    }

    #[test]
    fn ruby() {
        let e = extract(
            "ruby",
            "require 'json'\nmodule Auth\n  class SessionStore\n    TTL = 30\n    def self.build\n    end\n    def valid?(token)\n    end\n  end\nend\n",
        );
        let n = names(&e);
        assert!(n.contains(&("Auth", "module")));
        assert!(n.contains(&("SessionStore", "class")));
        assert!(n.contains(&("build", "method")));
        assert!(n.contains(&("valid?", "method")));
        assert!(n.contains(&("TTL", "constant")));
        assert_eq!(e.imports, vec!["json"]);
        assert_eq!(
            e.symbols.iter().find(|s| s.name == "valid?").unwrap().line,
            7
        );
    }

    #[test]
    fn python() {
        let e = extract(
            "python",
            "from app.models import User\nimport os\n\nclass Cart:\n    async def total(self):\n        pass\n\ndef apply_discount(cart):\n    pass\n",
        );
        let n = names(&e);
        assert!(n.contains(&("Cart", "class")));
        assert!(n.contains(&("total", "function")));
        assert!(n.contains(&("apply_discount", "function")));
        assert_eq!(e.imports, vec!["app.models", "os"]);
    }

    #[test]
    fn typescript() {
        let e = extract(
            "typescript",
            "import { x } from './util';\nexport interface Page { n: number }\nexport type Id = string;\nexport async function paginate<T>(items: T[]) {}\nexport const fmt = (a: number): string => `${a}`;\nclass Repo {\n  async findAll(limit: number): Promise<void> {\n  }\n}\n",
        );
        let n = names(&e);
        assert!(n.contains(&("Page", "interface")));
        assert!(n.contains(&("Id", "type")));
        assert!(n.contains(&("paginate", "function")));
        assert!(n.contains(&("fmt", "function")));
        assert!(n.contains(&("Repo", "class")));
        assert!(n.contains(&("findAll", "method")));
        assert!(!n.iter().any(|(name, _)| *name == "if"));
        assert_eq!(e.imports, vec!["./util"]);
    }

    #[test]
    fn rust_and_go() {
        let e = extract(
            "rust",
            "use std::fmt;\npub struct Index;\nimpl Display for Index {}\npub(crate) async fn refresh() {}\nconst MAX_FILES: usize = 3;\n",
        );
        let n = names(&e);
        assert!(n.contains(&("Index", "struct")));
        assert!(n.contains(&("Index", "impl")));
        assert!(n.contains(&("refresh", "function")));
        assert!(n.contains(&("MAX_FILES", "constant")));

        let e = extract(
            "go",
            "package auth\n\nimport (\n\t\"fmt\"\n\tlog \"github.com/x/log\"\n)\n\ntype Session struct {\n}\n\nfunc (s *Session) Valid() bool {\n\treturn true\n}\n\nfunc New() *Session {}\n",
        );
        let n = names(&e);
        assert!(n.contains(&("Session", "type")));
        assert!(n.contains(&("Valid", "method")));
        assert!(n.contains(&("New", "function")));
        assert!(e.imports.contains(&"fmt".to_string()));
        assert!(e.imports.contains(&"github.com/x/log".to_string()));
    }

    #[test]
    fn java() {
        let e = extract(
            "java",
            "package a;\nimport java.util.List;\npublic class OrderService {\n    public List<Order> findOrders(int page) {\n        if (x) {}\n    }\n}\n",
        );
        let n = names(&e);
        assert!(n.contains(&("OrderService", "class")));
        assert!(n.contains(&("findOrders", "method")));
        assert!(!n.iter().any(|(name, _)| *name == "if"));
    }
}
