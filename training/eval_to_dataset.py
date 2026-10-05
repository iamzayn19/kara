"""Convert solved `veyra eval` tasks into training examples.

Fixture repositories are written for this project (owner-created data), so
solved tasks can be used as examples. Each example pairs the task prompt with
the reference solution as an apply_patch step plus the check command.
"""

import argparse
import json
import os
import sys
import tomllib


def load_tasks(suite: str) -> dict:
    tasks = {}
    for name in sorted(os.listdir(suite)):
        f = os.path.join(suite, name, "veyra-tasks.toml")
        if not os.path.exists(f):
            continue
        with open(f, "rb") as fh:
            for t in tomllib.load(fh).get("task", []):
                t["fixture"] = os.path.join(suite, name)
                tasks[t["id"]] = t
    return tasks


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("reports", nargs="+")
    ap.add_argument("--suite", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    tasks = load_tasks(args.suite)
    seen = set()
    n = 0
    with open(args.out, "w", encoding="utf-8") as out:
        for report in args.reports:
            with open(report, encoding="utf-8") as fh:
                rep = json.load(fh)
            for r in rep.get("results", []):
                tid = r["task"]
                if not r.get("success") or tid in seen or tid not in tasks:
                    continue
                seen.add(tid)
                t = tasks[tid]
                patch_file = os.path.join(t["fixture"], "solutions", f"{tid}.patch")
                with open(patch_file, encoding="utf-8") as pf:
                    patch = pf.read()
                ex = {
                    "id": f"fixture-{tid}",
                    "source": "fixture_eval",
                    "license": "Apache-2.0",
                    "consent": {"explicit": True, "how": "owner-created fixture"},
                    "task": t["prompt"],
                    "language": os.path.basename(t["fixture"]).split("-")[0],
                    "steps": [
                        {"tool": "apply_patch", "arguments": json.dumps({"patch": patch}), "ok": True},
                        {"tool": "run_test", "arguments": json.dumps({"command": t["check"]}), "ok": True},
                    ],
                    "outcome": {"status": "completed", "tests_passed": True, "files_changed": t.get("expect_changed", [])},
                }
                out.write(json.dumps(ex) + "\n")
                n += 1
    print(f"wrote {n} examples -> {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
