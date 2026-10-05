"""Turn opt-in Veyra traces into training examples.

Reads ~/.veyra/traces/*.jsonl (written only when the user set
privacy.training_data = true), keeps completed turns whose last test passed,
redacts credentials, drops absolute paths, and writes dataset_schema.json
records. Standard library only.
"""

import argparse
import glob
import hashlib
import json
import os
import re
import sys

SECRET_PATTERNS = [
    re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----"),
    re.compile(r"\b(AKIA|ASIA)[0-9A-Z]{16}\b"),
    re.compile(r"\b(gh[pousr]_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{40,})\b"),
    re.compile(r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b"),
    re.compile(r"\bsk-[A-Za-z0-9_\-]{24,}\b"),
    re.compile(r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b"),
    re.compile(r"(?i)((?:password|passwd|secret|api[_-]?key|token)\w*\s*[:=]\s*[\"'])[^\"'\s]{8,}([\"'])"),
    re.compile(r"(\b[a-z][a-z0-9+.-]*://[^/\s:@]+:)[^/\s:@]{4,}(@)"),
]
ABS_PATH = re.compile(r"(?:/Users|/home|C:\\Users)[/\\][^/\\\s\"']+")


def redact(text: str) -> str:
    for p in SECRET_PATTERNS:
        if p.groups >= 2:
            text = p.sub(lambda m: f"{m.group(1)}[REDACTED]{m.group(m.lastindex)}", text)
        else:
            text = p.sub("[REDACTED]", text)
    return ABS_PATH.sub("<home>", text)


def convert(record: dict) -> dict | None:
    if record.get("schema") != "veyra-trace/1" or record.get("outcome") != "completed":
        return None
    tests = record.get("tests") or []
    if not tests or tests[-1].get("exit_code") != 0:
        return None
    steps = [
        {
            "tool": c.get("tool", ""),
            "arguments": redact(c.get("arguments", "")),
            "ok": bool(c.get("ok")),
            "summary": redact(c.get("summary", "")),
        }
        for c in record.get("tool_calls", [])
    ]
    task = redact(record.get("task", ""))
    uid = hashlib.sha256((record.get("started", "") + task).encode()).hexdigest()[:16]
    return {
        "id": f"trace-{uid}",
        "source": "opt_in_trace",
        "license": "user-provided",
        "consent": {"explicit": True, "how": "privacy.training_data = true"},
        "task": task,
        "steps": steps,
        "outcome": {
            "status": "completed",
            "tests_passed": True,
            "files_changed": sorted(record.get("files_changed", [])),
        },
        "model": record.get("model", ""),
    }


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--traces", default=os.path.expanduser("~/.veyra/traces"))
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    files = sorted(glob.glob(os.path.join(args.traces, "*.jsonl")))
    if not files:
        print(f"No traces in {args.traces}. Traces exist only if you opted in with privacy.training_data = true.")
        return 1
    kept = total = 0
    os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
    with open(args.out, "w", encoding="utf-8") as out:
        for f in files:
            with open(f, encoding="utf-8") as fh:
                for line in fh:
                    total += 1
                    try:
                        ex = convert(json.loads(line))
                    except json.JSONDecodeError:
                        continue
                    if ex:
                        out.write(json.dumps(ex) + "\n")
                        kept += 1
    print(f"kept {kept} of {total} traces -> {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
