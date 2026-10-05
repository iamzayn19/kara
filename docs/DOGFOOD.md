# Dogfooding: Kara on Kara

Kara is tested by using it on its own repository with a real local model.
Failures found this way become regression tests.

## Session 1, 2026-10-05: Qwen3-4B Q4_K_M, Apple M5 16 GB

Interactive `kara` in the repository root, driven through a terminal.

| Step | Result |
|---|---|
| Explain the architecture | First attempt was vague (the model never saw the docs). After the fix below, the answer named the crates and pointed to ARCHITECTURE.md. |
| Add a harmless feature: a unit test for `Profile::parse("AUTO")` | First attempts failed: the model could not express "insert new code" with search/replace, then **claimed success anyway**. After the fixes it inserted the test with `insert_lines`. |
| Run its own tests | Before running anything, the model claimed "16/16 passed". The verification gate made it actually run `cargo test -p kara-core`: 17 passed. |
| `/diff` | Showed Kara's change separately from the maintainer's own uncommitted edits to two docs files, made concurrently. |
| `/oracle` | Produced a structured review; some findings were plausible, some wrong (it flagged missing error handling that exists). Expected from a 4B model. |
| `/undo` | Restored `permissions.rs` exactly; the maintainer's concurrent uncommitted edits were untouched. |
| Verify repository state | `git status` showed only the maintainer's own edits. |

### Failures turned into fixes and regression tests

| Failure | Fix | Test |
|---|---|---|
| Model claimed an edit that never applied | When mutations were attempted but nothing changed, Kara pushes back once, then appends "no files were changed" to the answer. | `claiming_an_edit_that_never_applied_is_caught` |
| Model claimed success while its last test run failed (seen in evaluation) | "Still failing" gate (bounded), then an honest note. | `claiming_done_while_tests_fail_is_pushed_back`, `finishing_with_failing_tests_is_reported_honestly` |
| Model edited a test file to make it pass (evaluation) | System prompt: fix the code, not the tests. | Covered by eval task `python-import` |
| Small model cannot add new code via search/replace | New `insert_lines` tool; the edit failure hint points to it. | `insert_lines_after_a_line` |
| Overview questions found no relevant files | Orientation always lists top-level directories and documentation. | `orientation_points_to_project_docs` |
| Banner listed languages alphabetically | Sorted by file count. | manual |
| `llama-server` outlived a killed `kara` | Signal handlers, plus reaping of servers whose owner died. | `stale_servers_with_dead_owners_are_reaped` |
| `/diff` printed binary `.pyc` changes as text | Binary files are summarized by size. | manual |

### Still open

* With a 4B model, inserted code sometimes lands in the wrong scope (here:
  just outside the `tests` module; it still compiled and ran).
* Review quality (`/oracle`) depends heavily on model size. Re-run this
  session with the high-tier model on suitable hardware.
