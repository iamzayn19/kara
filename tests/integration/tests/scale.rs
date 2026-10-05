//! Repository scale: indexing must be incremental and orientation must stay
//! small regardless of repository size. The 100k-file case is ignored by
//! default (`cargo test -p veyra-integration-tests --test scale -- --ignored`).

use std::time::Instant;
use veyra_context::{orient, LanguageRegistry, RepoIndex};

fn generate(root: &std::path::Path, files: usize) {
    for i in 0..files {
        let dir = root.join(format!("pkg{}/sub{}", i % 97, i % 13));
        std::fs::create_dir_all(&dir).unwrap();
        let body = format!(
            "class Service{i}:\n    def handle_{i}(self, request):\n        return request\n\n\ndef helper_{i}():\n    return {i}\n"
        );
        std::fs::write(dir.join(format!("mod_{i}.py")), body).unwrap();
    }
    std::fs::write(
        root.join("pkg1/sub1/auth.py"),
        "class SessionStore:\n    def authenticate(self, token):\n        return token\n",
    )
    .unwrap();
}

fn run(files: usize, first_budget_s: f64) {
    let repo = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    generate(repo.path(), files);
    let idx = RepoIndex::open(repo.path(), cache.path(), LanguageRegistry::builtin(), 1_000_000).unwrap();

    let t = Instant::now();
    let s1 = idx.refresh().unwrap();
    let first = t.elapsed().as_secs_f64();
    assert_eq!(s1.files, files + 1);

    let t = Instant::now();
    let s2 = idx.refresh().unwrap();
    let second = t.elapsed().as_secs_f64();
    assert_eq!(s2.parsed, 0, "unchanged files are not re-parsed");

    let t = Instant::now();
    let o = orient(&idx, "fix authenticate in SessionStore", &[], 12).unwrap();
    let rank = t.elapsed().as_secs_f64();
    assert_eq!(o.files[0].path, "pkg1/sub1/auth.py");
    assert!(o.render().len() < 4000, "orientation stays small");

    eprintln!("{files} files: first index {first:.2}s, incremental {second:.2}s, rank {rank:.3}s");
    assert!(first < first_budget_s, "first index took {first:.1}s");
    assert!(second < first, "incremental refresh is faster than a full index");
}

#[test]
fn index_1k_files() {
    run(1_000, 30.0);
}

#[test]
fn index_10k_files() {
    run(10_000, 120.0);
}

#[test]
#[ignore]
fn index_100k_files() {
    run(100_000, 900.0);
}
