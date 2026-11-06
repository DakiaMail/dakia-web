import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import hashlib
import socket

spec = importlib.util.spec_from_file_location("release_check", Path(__file__).parents[1] / "release-check.py")
release = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = release
spec.loader.exec_module(release)


class RunnerTests(unittest.TestCase):
    def test_source_manifest_includes_uncommitted_content_and_deletions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "tracked").write_text("edited")
            (root / "new").write_text("new test")
            with patch.object(release.subprocess, "check_output", return_value=b"tracked\0new\0deleted\0tracked\0"):
                self.assertEqual(release.source_manifest(root), {
                    "tracked": hashlib.sha256(b"edited").hexdigest(),
                    "new": hashlib.sha256(b"new test").hexdigest(),
                    "deleted": "deleted",
                })

    def test_failure_does_not_suppress_independent_checks(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            stages = [release.Stage("build", ()), release.Stage("browser", (), requires=("build",)), release.Stage("storage", ())]
            called = []
            def execute(stage, *_):
                called.append(stage.name)
                return {"name": stage.name, "status": "failed" if stage.name == "build" else "passed"}
            results = release.run(stages, output, {}, execute)
            self.assertEqual(called, ["build", "storage"])
            self.assertEqual([r["status"] for r in results], ["failed", "blocked", "passed"])
            self.assertEqual(json.loads((output / "stages.json").read_text()), results)

    def test_nonzero_exit_is_never_a_pass(self):
        with tempfile.TemporaryDirectory() as directory:
            result = release.execute(release.Stage("failure", (sys.executable, "-c", "print('fixture'); raise SystemExit(4)")), Path(directory), {})
            self.assertEqual(result["exit_code"], 4)
            self.assertEqual(result["status"], "failed")
            self.assertIn("fixture", Path(result["log"]).read_text())

    def test_timeout_is_not_pass_or_skip(self):
        with tempfile.TemporaryDirectory() as directory:
            result = release.execute(release.Stage("timeout", (sys.executable, "-c", "import time; time.sleep(30)"), timeout=0.1), Path(directory), {})
            self.assertEqual(result["status"], "timed_out")

    def test_timeout_stops_a_detached_preview_server(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            child = output / "preview.py"
            port_file = output / "port"
            child.write_text("import socket,time,sys\nfrom pathlib import Path\ns=socket.socket()\ns.bind(('127.0.0.1',0))\ns.listen()\nPath(sys.argv[1]).write_text(str(s.getsockname()[1]))\ntime.sleep(30)\n")
            parent = "import subprocess,sys,time; subprocess.Popen([sys.executable,sys.argv[1],sys.argv[2]],start_new_session=True); time.sleep(30)"
            stage = release.Stage("detached", (sys.executable, "-c", parent, str(child), str(port_file)), timeout=1)
            result = release.execute(stage, output, {})
            self.assertEqual(result["status"], "timed_out")
            self.assertTrue(port_file.exists(), "fixture server must actually start")
            with socket.socket() as probe:
                probe.settimeout(1)
                self.assertNotEqual(probe.connect_ex(("127.0.0.1", int(port_file.read_text()))), 0)

    def test_missing_executable_is_blocked(self):
        with tempfile.TemporaryDirectory() as directory:
            result = release.execute(release.Stage("missing", ("/nonexistent/dakia-test-command",)), Path(directory), {})
            self.assertEqual(result["status"], "blocked")

    def test_full_profile_includes_real_backend_and_provider_checks(self):
        names = {stage.name for stage in release.stages("full", "1.98.0")}
        self.assertTrue({"rust", "web-unit", "web-build", "browser", "browser-backend", "browser-keyboard", "pwa", "pwa-lifecycle", "workers", "clippy", "format", "dovecot", "translation-models", "browser-full-text", "browser-translation", "browser-images", "browser-unsubscribe"} <= names)
        self.assertNotIn("dovecot", {stage.name for stage in release.stages("core", "1.98.0")})

    def test_remote_fixture_choice_only_changes_dovecot_stage(self):
        local = release.stages("full", "1.98.0")
        remote = release.stages("full", "1.98.0", "fixture-host")
        changed = [(a, b) for a, b in zip(local, remote) if a != b]
        self.assertEqual(len(changed), 1)
        self.assertEqual(changed[0][1].command[-2:], ("--ssh-host", "fixture-host"))

    def test_stages_have_distinct_browser_reports(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            for name in ["browser", "browser-backend", "browser-keyboard", "pwa", "pwa-lifecycle"]:
                command = (sys.executable, "-c", "import os; print(os.environ['DAKIA_BROWSER_REPORT'])")
                result = release.execute(release.Stage(name, command), output, {})
                self.assertEqual(Path(result["log"]).read_text().strip(), str(output / f"{name}.json"))

    def test_cargo_counts_include_failures_and_ignored_cases(self):
        with tempfile.TemporaryDirectory() as directory:
            text = "test result: FAILED. 10 passed; 2 failed; 1 ignored;\ntest result: ok. 4 passed; 0 failed; 3 ignored;"
            result = release.execute(release.Stage("counts", (sys.executable, "-c", f"print({text!r}); raise SystemExit(1)")), Path(directory), {})
            self.assertEqual(result["tests"], {"passed": 14, "failed": 2, "ignored": 4})
            self.assertEqual(result["status"], "failed")


if __name__ == "__main__":
    unittest.main()
