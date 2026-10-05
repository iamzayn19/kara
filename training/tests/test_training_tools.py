import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import check_hardware  # noqa: E402
import sanitize_traces  # noqa: E402


class SanitizeTest(unittest.TestCase):
    def record(self, **kw):
        r = {
            "schema": "veyra-trace/1",
            "started": "2026-01-01T00:00:00Z",
            "task": "fix login for /Users/alice/app",
            "outcome": "completed",
            "tool_calls": [{"tool": "read_file", "arguments": '{"path": "a.py", "token": "ghp_abcdefghijklmnopqrstuvwxyz0123456789"}', "ok": True, "summary": "ok"}],
            "files_changed": ["a.py"],
            "tests": [{"exit_code": 0}],
            "model": "qwen3-4b",
        }
        r.update(kw)
        return r

    def test_keeps_successful_and_redacts(self):
        ex = sanitize_traces.convert(self.record())
        self.assertIsNotNone(ex)
        blob = json.dumps(ex)
        self.assertNotIn("ghp_abcdef", blob)
        self.assertNotIn("/Users/alice", blob)
        self.assertTrue(ex["consent"]["explicit"])

    def test_drops_failures(self):
        self.assertIsNone(sanitize_traces.convert(self.record(outcome="stalled")))
        self.assertIsNone(sanitize_traces.convert(self.record(tests=[{"exit_code": 1}])))
        self.assertIsNone(sanitize_traces.convert(self.record(tests=[])))

    def test_cli_without_traces(self):
        with tempfile.TemporaryDirectory() as d:
            rc = sanitize_traces.main(["--traces", d, "--out", os.path.join(d, "o.jsonl")])
            self.assertEqual(rc, 1)


class HardwareTest(unittest.TestCase):
    def test_requirements_scale(self):
        self.assertLess(check_hardware.required_gb(1.7, "qlora"), check_hardware.required_gb(14, "qlora"))
        self.assertLess(check_hardware.required_gb(4, "qlora"), check_hardware.required_gb(4, "full"))
        self.assertEqual(check_hardware.params_from_name("Qwen/Qwen3-1.7B"), 1.7)

    def test_refuses_huge_full_finetune(self):
        rc = check_hardware.main(["--model", "x-35b", "--method", "full"])
        self.assertEqual(rc, 2)


if __name__ == "__main__":
    unittest.main()
