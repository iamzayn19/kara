//! `/undo` restores exactly what existed before Kara's last turn, keeps the
//! user's own uncommitted work, and never overwrites later user edits.

use serde_json::json;
use std::sync::Arc;
use kara_agent::approver::DenyAll;
use kara_core::permissions::Profile;
use kara_integration_tests::*;
use kara_model::scripted::{call, text, ScriptedProvider};
use kara_protocol::AgentMode;

#[tokio::test]
async fn undo_preserves_uncommitted_user_changes() {
    let repo = Repo::from_fixture("python-shop");
    // The user has work in progress in the same file Kara will edit, and in
    // another file Kara will not touch.
    let user_cart = repo.read("shop/cart.py").replace(
        "\"\"\"Shopping cart.\"\"\"",
        "\"\"\"Shopping cart (user WIP).\"\"\"",
    );
    repo.write("shop/cart.py", &user_cart);
    repo.write("README.md", "user notes\n");
    let before_status = git(repo.path(), &["status", "--porcelain"]);

    let provider = Arc::new(ScriptedProvider::new(vec![
        call("read_file", json!({"path": "shop/cart.py"})),
        call(
            "edit_file",
            json!({"path": "shop/cart.py", "old_string": "percent / 100)", "new_string": "(100 - percent) / 100)"}),
        ),
        call(
            "create_file",
            json!({"path": "shop/new_helper.py", "content": "def helper():\n    return 1\n"}),
        ),
        text("done"),
    ]));
    let mut h = Harness::new(&repo, provider, Profile::Balanced, Arc::new(DenyAll));
    h.agent.settings.verify_after_edit = false;
    let r = h.agent.run_turn("fix discount", AgentMode::Execute).await;
    assert_eq!(r.changed_files, vec!["shop/cart.py", "shop/new_helper.py"]);
    assert!(
        repo.read("shop/cart.py").contains("(user WIP)"),
        "edit kept the user's change"
    );
    assert!(repo.read("shop/cart.py").contains("(100 - percent)"));

    let report = h.agent.ctx.journal().undo(None).unwrap();
    assert!(report.conflicts.is_empty());
    assert_eq!(
        repo.read("shop/cart.py"),
        user_cart,
        "exact pre-turn content, including user WIP"
    );
    assert_eq!(
        repo.read("README.md"),
        "user notes\n",
        "unrelated user change untouched"
    );
    assert!(!repo.path().join("shop/new_helper.py").exists());
    assert_eq!(git(repo.path(), &["status", "--porcelain"]), before_status);
}

#[tokio::test]
async fn undo_skips_files_the_user_edited_afterwards() {
    let repo = Repo::from_fixture("python-shop");
    let provider = Arc::new(ScriptedProvider::new(vec![
        call(
            "edit_file",
            json!({"path": "shop/cart.py", "old_string": "percent / 100)", "new_string": "(100 - percent) / 100)"}),
        ),
        call(
            "edit_file",
            json!({"path": "shop/format.py", "old_string": "\"\"\"Formatting helpers.\"\"\"", "new_string": "\"\"\"Money formatting.\"\"\""}),
        ),
        text("done"),
    ]));
    let mut h = Harness::new(&repo, provider, Profile::Balanced, Arc::new(DenyAll));
    h.agent.settings.verify_after_edit = false;
    h.agent.run_turn("edit two files", AgentMode::Execute).await;

    // User edits one of them after Kara.
    let mine = repo.read("shop/format.py") + "# my follow-up edit\n";
    repo.write("shop/format.py", &mine);

    let report = h.agent.ctx.journal().undo(None).unwrap();
    assert_eq!(report.conflicts, vec!["shop/format.py"]);
    assert_eq!(report.restored, vec!["shop/cart.py"]);
    assert_eq!(repo.read("shop/format.py"), mine, "user edit preserved");
    assert!(repo.read("shop/cart.py").contains("percent / 100)"));
}

#[tokio::test]
async fn each_turn_is_a_separate_undo_batch() {
    let repo = Repo::from_fixture("python-shop");
    let provider = Arc::new(ScriptedProvider::new(vec![
        call(
            "edit_file",
            json!({"path": "shop/cart.py", "old_string": "\"\"\"Shopping cart.\"\"\"", "new_string": "\"\"\"Cart v1.\"\"\""}),
        ),
        text("one"),
        call(
            "edit_file",
            json!({"path": "shop/cart.py", "old_string": "\"\"\"Cart v1.\"\"\"", "new_string": "\"\"\"Cart v2.\"\"\""}),
        ),
        text("two"),
    ]));
    let mut h = Harness::new(&repo, provider, Profile::Balanced, Arc::new(DenyAll));
    h.agent.settings.verify_after_edit = false;
    h.agent.run_turn("first", AgentMode::Execute).await;
    h.agent.run_turn("second", AgentMode::Execute).await;
    h.agent.ctx.journal().undo(None).unwrap();
    assert!(repo.read("shop/cart.py").contains("Cart v1."));
    h.agent.ctx.journal().undo(None).unwrap();
    assert!(repo.read("shop/cart.py").contains("Shopping cart."));
    assert_eq!(git(repo.path(), &["status", "--porcelain"]), "");
}
