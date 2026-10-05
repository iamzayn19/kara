//! Extract pass/fail counts and failing test names from common test runners.
//! Parsing is best-effort; the exit code remains the source of truth.

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedTests {
    pub passed: Option<u32>,
    pub failed: Option<u32>,
    pub failed_tests: Vec<String>,
}

struct Patterns {
    rspec: Regex,
    rspec_fail: Regex,
    minitest: Regex,
    minitest_fail: Regex,
    pytest_summary: Regex,
    pytest_fail: Regex,
    unittest_ran: Regex,
    unittest_failed: Regex,
    unittest_fail: Regex,
    cargo: Regex,
    cargo_fail: Regex,
    go_fail: Regex,
    go_pass: Regex,
    jest: Regex,
    vitest: Regex,
    jest_fail: Regex,
    node_pass: Regex,
    node_fail: Regex,
    node_fail_name: Regex,
    junit: Regex,
    junit_fail: Regex,
    dotnet: Regex,
    phpunit: Regex,
}

fn p() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| {
        let r = |s: &str| Regex::new(s).expect("valid test regex");
        Patterns {
            rspec: r(r"(\d+) examples?, (\d+) failures?"),
            rspec_fail: r(r"(?m)^rspec (\./\S+)"),
            minitest: r(r"(\d+) (?:runs|tests), \d+ assertions, (\d+) failures, (\d+) errors"),
            minitest_fail: r(r"(?m)^\s*\d+\) (?:Failure|Error):\s*\n?\s*([\w:]+#\w+)"),
            pytest_summary: r(r"(?m)^=+ (.*?(?:passed|failed|error).*?) in [\d.]+s"),
            pytest_fail: r(r"(?m)^(?:FAILED|ERROR) (\S+)"),
            unittest_ran: r(r"(?m)^Ran (\d+) tests?"),
            unittest_failed: r(r"(?m)^FAILED \((?:failures=(\d+))?(?:, )?(?:errors=(\d+))?"),
            unittest_fail: r(r"(?m)^(?:FAIL|ERROR): (\w+) \(([\w.]+)\)"),
            cargo: r(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed"),
            cargo_fail: r(r"(?m)^test (\S+) \.\.\. FAILED"),
            go_fail: r(r"(?m)^\s*--- FAIL: (\S+)"),
            go_pass: r(r"(?m)^\s*--- PASS: "),
            jest: r(r"(?m)^Tests:\s+(?:(\d+) failed, )?(?:\d+ skipped, )?(?:(\d+) passed, )?\d+ total"),
            vitest: r(r"(?m)^\s*Tests\s+(?:(\d+) failed)?\s*\|?\s*(?:(\d+) passed)?"),
            jest_fail: r(r"(?m)^\s*(?:✕|×|✗)\s+(.+?)(?:\s+\(\d+ ?ms\))?$"),
            node_pass: r(r"(?m)^# pass (\d+)"),
            node_fail: r(r"(?m)^# fail (\d+)"),
            node_fail_name: r(r"(?m)^\s*not ok \d+ - (.+)$"),
            junit: r(r"Tests run: (\d+), Failures: (\d+), Errors: (\d+)"),
            junit_fail: r(r"(?m)^\[ERROR\]\s+(\S+)\s+(?:Time elapsed|<<<|.*FAILURE)"),
            dotnet: r(r"(?:Passed|Failed)!\s+-\s+Failed:\s+(\d+),\s+Passed:\s+(\d+)"),
            phpunit: r(r"Tests: (\d+), Assertions: \d+(?:, (?:Failures|Errors): (\d+))?"),
        }
    })
}

pub fn parse(output: &str) -> ParsedTests {
    let p = p();
    let mut t = ParsedTests::default();
    let mut names: Vec<String> = Vec::new();

    if let Some(c) = p.rspec.captures(output) {
        let total: u32 = c[1].parse().unwrap_or(0);
        let failed: u32 = c[2].parse().unwrap_or(0);
        t.failed = Some(failed);
        t.passed = Some(total.saturating_sub(failed));
        names.extend(p.rspec_fail.captures_iter(output).map(|c| c[1].to_string()));
    } else if let Some(c) = p.minitest.captures(output) {
        let total: u32 = c[1].parse().unwrap_or(0);
        let failed: u32 = c[2].parse::<u32>().unwrap_or(0) + c[3].parse::<u32>().unwrap_or(0);
        t.failed = Some(failed);
        t.passed = Some(total.saturating_sub(failed));
        names.extend(p.minitest_fail.captures_iter(output).map(|c| c[1].to_string()));
    } else if let Some(c) = p.pytest_summary.captures(output) {
        let summary = &c[1];
        let num = |word: &str| -> Option<u32> {
            Regex::new(&format!(r"(\d+) {word}"))
                .ok()?
                .captures(summary)
                .and_then(|c| c[1].parse().ok())
        };
        t.passed = Some(num("passed").unwrap_or(0));
        t.failed = Some(num("failed").unwrap_or(0) + num("errors?").unwrap_or(0));
        names.extend(p.pytest_fail.captures_iter(output).map(|c| c[1].to_string()));
    } else if let Some(c) = p.unittest_ran.captures(output) {
        let total: u32 = c[1].parse().unwrap_or(0);
        let failed = p
            .unittest_failed
            .captures(output)
            .map(|c| {
                c.get(1).and_then(|m| m.as_str().parse::<u32>().ok()).unwrap_or(0)
                    + c.get(2).and_then(|m| m.as_str().parse::<u32>().ok()).unwrap_or(0)
            })
            .unwrap_or(0);
        t.failed = Some(failed);
        t.passed = Some(total.saturating_sub(failed));
        names.extend(
            p.unittest_fail
                .captures_iter(output)
                .map(|c| format!("{}.{}", &c[2], &c[1])),
        );
    } else if p.cargo.is_match(output) {
        let (mut passed, mut failed) = (0, 0);
        for c in p.cargo.captures_iter(output) {
            passed += c[1].parse::<u32>().unwrap_or(0);
            failed += c[2].parse::<u32>().unwrap_or(0);
        }
        t.passed = Some(passed);
        t.failed = Some(failed);
        names.extend(p.cargo_fail.captures_iter(output).map(|c| c[1].to_string()));
    } else if p.go_fail.is_match(output) || p.go_pass.is_match(output) || output.contains("\nok  \t") {
        let failed: Vec<String> = p.go_fail.captures_iter(output).map(|c| c[1].to_string()).collect();
        t.failed = Some(failed.len() as u32);
        t.passed = Some(p.go_pass.find_iter(output).count() as u32);
        names.extend(failed);
    } else if let Some(c) = p.jest.captures(output) {
        t.failed = Some(c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0));
        t.passed = Some(c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0));
        names.extend(p.jest_fail.captures_iter(output).map(|c| c[1].trim().to_string()));
    } else if p.node_pass.is_match(output) {
        t.passed = p.node_pass.captures(output).and_then(|c| c[1].parse().ok());
        t.failed = p.node_fail.captures(output).and_then(|c| c[1].parse().ok());
        names.extend(p.node_fail_name.captures_iter(output).map(|c| c[1].trim().to_string()));
    } else if p.junit.is_match(output) {
        // Maven prints per-class lines and a final total; the last one is the total.
        if let Some(c) = p.junit.captures_iter(output).last() {
            let total: u32 = c[1].parse().unwrap_or(0);
            let failed = c[2].parse::<u32>().unwrap_or(0) + c[3].parse::<u32>().unwrap_or(0);
            t.failed = Some(failed);
            t.passed = Some(total.saturating_sub(failed));
        }
        names.extend(p.junit_fail.captures_iter(output).map(|c| c[1].to_string()));
    } else if let Some(c) = p.dotnet.captures(output) {
        t.failed = c[1].parse().ok();
        t.passed = c[2].parse().ok();
    } else if let Some(c) = p.phpunit.captures(output) {
        let total: u32 = c[1].parse().unwrap_or(0);
        let failed: u32 = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        t.failed = Some(failed);
        t.passed = Some(total.saturating_sub(failed));
    } else if let Some(c) = p.vitest.captures(output) {
        if c.get(1).is_some() || c.get(2).is_some() {
            t.failed = Some(c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0));
            t.passed = Some(c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0));
            names.extend(p.jest_fail.captures_iter(output).map(|c| c[1].trim().to_string()));
        }
    }

    names.dedup();
    names.truncate(30);
    t.failed_tests = names;
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rspec() {
        let out = "....F\n\nFailures:\n...\n18 examples, 2 failures\n\nFailed examples:\n\nrspec ./spec/auth_spec.rb:12 # Auth logs in\nrspec ./spec/auth_spec.rb:30 # Auth logs out\n";
        let t = parse(out);
        assert_eq!(t.passed, Some(16));
        assert_eq!(t.failed, Some(2));
        assert_eq!(t.failed_tests, vec!["./spec/auth_spec.rb:12", "./spec/auth_spec.rb:30"]);
    }

    #[test]
    fn minitest() {
        let out = "  1) Failure:\nCartTest#test_total [test/cart_test.rb:9]:\nExpected: 10\n\n5 runs, 7 assertions, 1 failures, 0 errors, 0 skips\n";
        let t = parse(out);
        assert_eq!((t.passed, t.failed), (Some(4), Some(1)));
        assert_eq!(t.failed_tests, vec!["CartTest#test_total"]);
    }

    #[test]
    fn pytest() {
        let out = "FAILED tests/test_cart.py::test_discount - assert 90 == 80\n========= 1 failed, 11 passed in 0.12s =========\n";
        let t = parse(out);
        assert_eq!((t.passed, t.failed), (Some(11), Some(1)));
        assert_eq!(t.failed_tests, vec!["tests/test_cart.py::test_discount"]);
        let t = parse("============ 3 passed in 0.01s ============\n");
        assert_eq!((t.passed, t.failed), (Some(3), Some(0)));
    }

    #[test]
    fn unittest() {
        let out = "FAIL: test_total (test_cart.CartTest)\n----\nRan 4 tests in 0.001s\n\nFAILED (failures=1)\n";
        let t = parse(out);
        assert_eq!((t.passed, t.failed), (Some(3), Some(1)));
        assert_eq!(t.failed_tests, vec!["test_cart.CartTest.test_total"]);
    }

    #[test]
    fn cargo() {
        let out = "test auth::tests::login ... ok\ntest auth::tests::expiry ... FAILED\n\ntest result: FAILED. 7 passed; 1 failed; 0 ignored\n\ntest result: ok. 2 passed; 0 failed; 0 ignored\n";
        let t = parse(out);
        assert_eq!((t.passed, t.failed), (Some(9), Some(1)));
        assert_eq!(t.failed_tests, vec!["auth::tests::expiry"]);
    }

    #[test]
    fn go() {
        let out = "=== RUN   TestLogin\n--- PASS: TestLogin (0.00s)\n=== RUN   TestExpiry\n--- FAIL: TestExpiry (0.00s)\nFAIL\nFAIL\texample.com/auth\t0.01s\n";
        let t = parse(out);
        assert_eq!((t.passed, t.failed), (Some(1), Some(1)));
        assert_eq!(t.failed_tests, vec!["TestExpiry"]);
    }

    #[test]
    fn jest_and_node() {
        let out = "  ✕ paginates results (5 ms)\nTests:       1 failed, 4 passed, 5 total\n";
        let t = parse(out);
        assert_eq!((t.passed, t.failed), (Some(4), Some(1)));
        assert_eq!(t.failed_tests, vec!["paginates results"]);
        let out = "not ok 2 - paginate returns second page\n# tests 3\n# pass 2\n# fail 1\n";
        let t = parse(out);
        assert_eq!((t.passed, t.failed), (Some(2), Some(1)));
        assert_eq!(t.failed_tests, vec!["paginate returns second page"]);
    }

    #[test]
    fn junit_and_dotnet() {
        let out = "Tests run: 3, Failures: 1, Errors: 0, Skipped: 0\n[ERROR] com.ex.OrderTest.testTotal  Time elapsed: 0.01 s  <<< FAILURE!\nTests run: 10, Failures: 1, Errors: 1, Skipped: 0\n";
        let t = parse(out);
        assert_eq!((t.passed, t.failed), (Some(8), Some(2)));
        let t = parse("Failed!  - Failed:     2, Passed:    10, Skipped: 0, Total: 12\n");
        assert_eq!((t.passed, t.failed), (Some(10), Some(2)));
    }

    #[test]
    fn unknown_output() {
        assert_eq!(parse("hello world"), ParsedTests::default());
    }
}
