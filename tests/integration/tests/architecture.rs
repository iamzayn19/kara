//! Architectural rules, checked mechanically.
//!
//! Kara Core (agent, tools, context, sandbox, configuration, protocol) must
//! not depend on a specific model, model size, GPU vendor, CUDA, Metal or
//! llama.cpp. Those live behind the inference interface in `kara-inference`.

use std::path::{Path, PathBuf};

const CORE_CRATES: &[&str] = &[
    "kara-protocol",
    "kara-core",
    "kara-sandbox",
    "kara-context",
    "kara-tools",
    "kara-agent",
];

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            out.push(p);
        }
    }
}

#[test]
fn core_crates_do_not_depend_on_specific_inference_backends() {
    let forbidden = regex::Regex::new(
        r"(?i)llama[-_.]?cpp|llama-server|llamacpp|\bcuda\b|\bmetal\b|\brocm\b|\bvulkan\b|\bqwen|kara_inference::local|HardwareInfo|\bgguf\b",
    )
    .unwrap();
    let mut violations = Vec::new();
    for c in CORE_CRATES {
        let mut files = Vec::new();
        rust_files(&crates_dir().join(c).join("src"), &mut files);
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            for (i, line) in text.lines().enumerate() {
                if forbidden.is_match(line) {
                    violations.push(format!("{}:{}: {}", f.display(), i + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "inference specifics in Kara Core:\n{}",
        violations.join("\n")
    );
}

#[test]
fn only_the_agent_sees_the_inference_interface_and_nothing_sees_a_runtime_crate() {
    for c in CORE_CRATES {
        let manifest = std::fs::read_to_string(crates_dir().join(c).join("Cargo.toml")).unwrap();
        let uses_inference = manifest.contains("kara-inference");
        assert_eq!(
            uses_inference,
            *c == "kara-agent",
            "{c}: only kara-agent may depend on kara-inference (for the provider trait)"
        );
    }
}
