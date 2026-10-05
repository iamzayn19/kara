//! The full agent loop over real fixture repositories with a scripted model:
//! prompt -> search -> file selection -> edit -> test -> failure recovery ->
//! passing test -> summary.

use serde_json::json;
use std::sync::Arc;
use veyra_agent::approver::{ApproveOrdinary, DenyAll};
use veyra_agent::AgentSettings;
use veyra_core::permissions::Profile;
use veyra_integration_tests::*;
use veyra_model::scripted::{call, text, ScriptedProvider};
use veyra_model::{ChatRequest, ChatResponse, Role};
use veyra_protocol::{AgentEvent, AgentMode, TurnOutcome};

fn last_tool_result(req: &ChatRequest) -> String {
    req.messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

#[tokio::test]
async fn fixes_bug_with_failure_recovery() {
    if !have(python(), "--version") {
        return;
    }
    let repo = Repo::from_fixture("python-shop");
    let script = vec![
        call("update_plan", json!({"steps": [{"title": "find discount code", "status": "in_progress"}, {"title": "fix and test"}], "hypothesis": "discount math is wrong"})),
        call("grep", json!({"pattern": "def apply_discount"})),
        call("read_file", json!({"path": "shop/cart.py"})),
        // First attempt is wrong on purpose (still discounts incorrectly).
        call("edit_file", json!({"path": "shop/cart.py", "old_string": "return round(self.total() * percent / 100)", "new_string": "return round(self.total() - percent)"})),
        call("run_test", json!({"files": ["test/test_cart.py"]})),
        // Recover: correct fix.
        call("edit_file", json!({"path": "shop/cart.py", "old_string": "return round(self.total() - percent)", "new_string": "return round(self.total() * (100 - percent) / 100)"})),
        call("run_test", json!({"files": ["test/test_cart.py"]})),
        call("git_diff", json!({})),
        text("Fixed `Cart.apply_discount` in shop/cart.py: it returned the discount amount instead of the discounted total. test/test_cart.py passes."),
    ];
    let provider = Arc::new(ScriptedProvider::new(script));
    let requests = provider.requests();
    let mut h = Harness::new(&repo, provider, Profile::Balanced, Arc::new(DenyAll));
    let r = h
        .agent
        .run_turn(
            "The cart discount tests are failing. Find the bug and fix it.",
            AgentMode::Execute,
        )
        .await;

    assert_eq!(r.outcome, TurnOutcome::Completed, "{}", r.summary);
    assert_eq!(r.changed_files, vec!["shop/cart.py"]);
    assert_eq!(r.stats.tests_run, 2);
    assert!(r.last_test.as_ref().unwrap().succeeded());
    assert_eq!(
        h.agent.state.retries, 1,
        "one failed verification before recovery"
    );
    assert_eq!(r.stats.invalid_tool_calls, 0);

    // The real check passes in the repository.
    let (ok, out) = repo.run("python3 -m unittest test.test_cart");
    assert!(ok || cfg!(windows), "{out}");

    // The failing run was reported to the model with failure details.
    let reqs = requests.lock().unwrap();
    let after_first_test = last_tool_result(&reqs[5]);
    assert!(after_first_test.contains("FAILED"), "{after_first_test}");
    assert!(
        after_first_test.contains("Working memory"),
        "working memory is attached"
    );
    assert!(
        after_first_test.contains("discount math is wrong"),
        "hypothesis is in working memory"
    );

    // Orientation pointed at the right file without dumping the repo.
    let first_user = &reqs[0]
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .unwrap()
        .content;
    assert!(first_user.contains("shop/cart.py"), "{first_user}");
    assert!(
        !first_user.contains("def total(self)"),
        "file contents are not dumped into context"
    );

    let events = h.events();
    let tests: Vec<bool> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TestFinished { report } => Some(report.succeeded()),
            _ => None,
        })
        .collect();
    assert_eq!(tests, vec![false, true]);
    assert!(events
        .iter()
        .any(|e| matches!(e, AgentEvent::FileChanged { .. })));
    assert!(matches!(
        events.last(),
        Some(AgentEvent::TurnFinished {
            outcome: TurnOutcome::Completed,
            ..
        })
    ));
}

#[tokio::test]
async fn verification_gate_requires_tests_after_edits() {
    if !have(python(), "--version") {
        return;
    }
    let repo = Repo::from_fixture("python-shop");
    let provider = Arc::new(ScriptedProvider::from_fn(|req: &ChatRequest, n| match n {
        0 => call("read_file", json!({"path": "shop/report.py"})),
        1 => call(
            "edit_file",
            json!({"path": "shop/report.py", "old_string": "from shop.format import money", "new_string": "from shop.format import format_money as money"}),
        ),
        // Tries to finish without testing.
        2 => text("Done."),
        _ => {
            let last = req.messages.last().unwrap();
            if last.role == Role::User && last.content.contains("have not run tests") {
                call("run_test", json!({"files": ["test/test_report.py"]}))
            } else {
                text("Fixed the import in shop/report.py; test_report passes.")
            }
        }
    }));
    let mut h = Harness::new(&repo, provider, Profile::Balanced, Arc::new(DenyAll));
    let r = h
        .agent
        .run_turn(
            "test_report fails with an import error. Fix it.",
            AgentMode::Execute,
        )
        .await;
    assert_eq!(r.outcome, TurnOutcome::Completed);
    assert_eq!(r.stats.tests_run, 1, "the gate forced a test run");
    assert!(r.last_test.unwrap().succeeded());
}

#[tokio::test]
async fn recovery_is_bounded_and_reported() {
    if !have(python(), "--version") {
        return;
    }
    let repo = Repo::from_fixture("python-shop");
    let provider = Arc::new(ScriptedProvider::from_fn(|_: &ChatRequest, n| {
        if n == 0 {
            call(
                "edit_file",
                json!({"path": "shop/cart.py", "old_string": "percent / 100)", "new_string": "percent / 100) + 0"}),
            )
        } else {
            // Keeps running the failing tests without fixing anything.
            call(
                "run_test",
                json!({"files": ["test/test_cart.py"], "timeout_secs": 60 + n}),
            )
        }
    }));
    let settings = AgentSettings {
        max_recovery_attempts: 2,
        ..Default::default()
    };
    let mut h = Harness::with_settings(
        &repo,
        provider,
        Profile::Balanced,
        Arc::new(DenyAll),
        settings,
    );
    let r = h
        .agent
        .run_turn("fix the discount", AgentMode::Execute)
        .await;
    assert_eq!(r.outcome, TurnOutcome::Stalled);
    assert!(r.summary.contains("max_recovery_attempts"), "{}", r.summary);
    assert_eq!(r.stats.tests_run, 3);
}

#[tokio::test]
async fn repeated_identical_calls_stall_instead_of_looping_forever() {
    let repo = Repo::from_fixture("python-shop");
    let provider = Arc::new(ScriptedProvider::from_fn(|_: &ChatRequest, _| {
        call("grep", json!({"pattern": "nothing_matches_this"}))
    }));
    let mut h = Harness::new(&repo, provider, Profile::Balanced, Arc::new(DenyAll));
    let r = h.agent.run_turn("find it", AgentMode::Execute).await;
    assert_eq!(r.outcome, TurnOutcome::Stalled);
    assert!(r.stats.tool_calls <= 7, "{}", r.stats.tool_calls);
}

#[tokio::test]
async fn invalid_tool_calls_are_reported_back_and_counted() {
    let repo = Repo::from_fixture("python-shop");
    let provider = Arc::new(ScriptedProvider::new(vec![
        ChatResponse {
            tool_calls: vec![veyra_model::ToolCall {
                id: "a".into(),
                name: "read_file".into(),
                arguments: "{not json".into(),
            }],
            ..Default::default()
        },
        call("teleport", json!({})),
        text("I could not complete the request."),
    ]));
    let requests = provider.requests();
    let mut h = Harness::new(&repo, provider, Profile::Balanced, Arc::new(DenyAll));
    let r = h.agent.run_turn("look around", AgentMode::Execute).await;
    assert_eq!(r.stats.invalid_tool_calls, 2);
    let reqs = requests.lock().unwrap();
    assert!(last_tool_result(&reqs[1]).contains("not valid JSON"));
    assert!(last_tool_result(&reqs[2]).contains("unknown tool `teleport`"));
}

#[tokio::test]
async fn plan_mode_is_read_only_and_waits_for_approval() {
    if !have(python(), "--version") {
        return;
    }
    let repo = Repo::from_fixture("python-shop");
    let provider = Arc::new(ScriptedProvider::new(vec![
        call("find_symbol", json!({"name": "apply_discount"})),
        call("edit_file", json!({"path": "shop/cart.py", "old_string": "percent / 100)", "new_string": "(100 - percent) / 100)"})),
        text("## Understanding\nDiscount returns the wrong amount.\n## Files likely to change\n- shop/cart.py\n## Tests to run\n- test/test_cart.py\nApprove?"),
        // After approval: execute.
        call("edit_file", json!({"path": "shop/cart.py", "old_string": "return round(self.total() * percent / 100)", "new_string": "return round(self.total() * (100 - percent) / 100)"})),
        call("run_test", json!({"files": ["test/test_cart.py"]})),
        text("Implemented the plan; tests pass."),
    ]));
    let requests = provider.requests();
    let mut h = Harness::new(
        &repo,
        provider,
        Profile::Autonomous,
        Arc::new(ApproveOrdinary),
    );
    let r = h
        .agent
        .run_turn("Fix the discount bug", AgentMode::Plan)
        .await;
    assert_eq!(r.outcome, TurnOutcome::AwaitingApproval);
    assert!(r.changed_files.is_empty());
    assert!(
        repo.read("shop/cart.py").contains("percent / 100)"),
        "plan mode must not edit"
    );
    {
        let reqs = requests.lock().unwrap();
        assert!(last_tool_result(&reqs[2]).contains("not available in plan mode"));
        let tool_names: Vec<&str> = reqs[0].tools.iter().map(|t| t.name.as_str()).collect();
        assert!(!tool_names.contains(&"edit_file") && tool_names.contains(&"read_file"));
    }
    assert!(h.agent.pending_plan().is_some());

    let r = h.agent.approve_plan().await.unwrap();
    assert_eq!(r.outcome, TurnOutcome::Completed);
    assert_eq!(r.changed_files, vec!["shop/cart.py"]);
    assert!(h.agent.pending_plan().is_none());
}

#[tokio::test]
async fn review_mode_receives_the_diff() {
    let repo = Repo::from_fixture("python-shop");
    repo.write(
        "shop/format.py",
        "def format_money(cents):\n    return str(cents)\n",
    );
    let provider = Arc::new(ScriptedProvider::new(vec![text(
        "## High\n- shop/format.py:2: format_money drops currency formatting; test/test_report.py will fail.\nOverall risk: high.",
    )]));
    let requests = provider.requests();
    let mut h = Harness::new(&repo, provider, Profile::Balanced, Arc::new(DenyAll));
    let r = h
        .agent
        .run_turn("Review my current diff.", AgentMode::Review)
        .await;
    assert_eq!(r.outcome, TurnOutcome::Completed);
    let reqs = requests.lock().unwrap();
    let user = &reqs[0]
        .messages
        .iter()
        .find(|m| m.role == Role::User)
        .unwrap()
        .content;
    assert!(
        user.contains("-    sign = \"-\" if cents < 0 else \"\""),
        "diff included: {user}"
    );
    assert!(reqs[0].messages[0].content.contains("Oracle"));
}

#[tokio::test]
async fn context_is_compacted_on_long_tasks() {
    let repo = Repo::from_fixture("python-shop");
    let provider = Arc::new(ScriptedProvider::from_fn(|_: &ChatRequest, n| {
        if n < 12 {
            // Distinct reads so the stall guard does not trigger.
            call(
                "read_range",
                json!({"path": "shop/cart.py", "start_line": 1, "end_line": 30 + n}),
            )
        } else {
            text("done")
        }
    }));
    let requests = provider.requests();
    let settings = AgentSettings {
        context_window: 4000,
        ..Default::default()
    };
    let mut h = Harness::with_settings(
        &repo,
        provider,
        Profile::Balanced,
        Arc::new(DenyAll),
        settings,
    );
    let r = h.agent.run_turn("read a lot", AgentMode::Execute).await;
    assert_eq!(r.outcome, TurnOutcome::Completed);
    let reqs = requests.lock().unwrap();
    let last = reqs.last().unwrap();
    assert!(
        last.messages
            .iter()
            .any(|m| m.content.starts_with("[elided")),
        "older tool output was elided"
    );
}
