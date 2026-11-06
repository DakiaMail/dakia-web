#!/usr/bin/env python3
"""Run release checks without hiding failures or touching owner mail.

Install dependencies first. The full profile requires Chrome and Docker.
Every stage gets a transcript and the run gets a machine-readable summary.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time
import tomllib

ROOT = Path(__file__).resolve().parents[1]


@dataclass(frozen=True)
class Stage:
    name: str
    command: tuple[str, ...]
    timeout: int = 600
    requires: tuple[str, ...] = ()


def source_manifest(root: Path = ROOT) -> dict[str, str]:
    paths = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=root).decode().split("\0")
    result = {}
    for name in sorted(set(filter(None, paths))):
        path = root / name
        if path.is_symlink():
            result[name] = "symlink:" + hashlib.sha256(os.readlink(path).encode()).hexdigest()
        elif path.is_file():
            with path.open("rb") as stream:
                result[name] = hashlib.file_digest(stream, "sha256").hexdigest()
        else:
            result[name] = "deleted"
    return result


def stages(profile: str, channel: str, dovecot_ssh: str | None = None) -> list[Stage]:
    cargo = ("cargo", f"+{channel}")
    result = [
        Stage("runner", (sys.executable, "-m", "unittest", "discover", "-s", "scripts/tests", "-v")),
        Stage("format", (*cargo, "fmt", "--all", "--", "--check")),
        Stage("rust", (*cargo, "test", "--workspace", "--locked", "--no-fail-fast"), 1200),
        Stage("clippy", (*cargo, "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"), 1200),
        Stage("web-unit", ("npm", "test")),
        Stage("web-build", ("npm", "run", "build")),
        Stage("workers", ("node", "--test", *map(str, sorted(Path("workers").glob("*/test/*.test.mjs"))))),
    ]
    if profile == "full":
        for name, config in [("browser", "release"), ("browser-backend", "backend"), ("browser-full-text", "full-text"), ("browser-translation", "translation"), ("browser-images", "images"), ("browser-unsubscribe", "unsubscribe-backend"), ("browser-cleanup", "silent-cleanup"), ("pwa", "pwa"), ("pwa-lifecycle", "lifecycle")]:
            result.append(Stage(name, ("npm", "run", "test:browser", "--workspace", "@dakia/web", "--", "--config", f"playwright.{config}.config.ts"), 1500, ("web-build",)))
        result.append(Stage("dovecot", (sys.executable, "crates/mail-provider/fixtures/run_dovecot.py", *(("--ssh-host", dovecot_ssh) if dovecot_ssh else ())), 900))
        result.append(Stage("translation-models", ("node", "--test", "workers/translation/test/models.test.mjs"), 900))
        for name, test in [("idle-push", "disposable_idle_to_push_pipeline"), ("thread-benchmark", "investigation_thread_scaling"), ("scheduler-benchmark", "cold_scheduler_storage_benchmark")]:
            result.append(Stage(name, (*cargo, "test", "-p", "dakia-server", "--lib", test, "--", "--ignored", "--nocapture"), 400))
    return result


def stop_tree(process: subprocess.Popen) -> None:
    # Playwright starts its web server in a separate session. Killing only the
    # test process group leaves that preview alive and blocks the next release.
    rows = subprocess.check_output(["ps", "-axo", "pid=,ppid=,pgid="], text=True)
    tree = [tuple(map(int, row.split())) for row in rows.splitlines() if row.strip()]
    owned = {process.pid}
    while True:
        children = {pid for pid, parent, _ in tree if parent in owned}
        if children <= owned:
            break
        owned.update(children)
    groups = {group for pid, _, group in tree if pid in owned}
    groups.discard(os.getpgrp())
    for group in groups:
        try:
            os.killpg(group, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    # Still-owned groups may outlive their parent after ignoring SIGTERM.
    for group in groups:
        try:
            os.killpg(group, signal.SIGKILL)
        except ProcessLookupError:
            pass
    process.wait()


def execute(stage: Stage, output: Path, env: dict[str, str], cwd: Path = ROOT) -> dict:
    started = time.monotonic()
    log = output / f"{stage.name}.log"
    result = {"name": stage.name, "command": list(stage.command), "log": str(log), "status": "failed"}
    with log.open("w") as stream:
        try:
            stage_env = dict(env, DAKIA_BROWSER_REPORT=str(output / f"{stage.name}.json"))
            if stage.name == "translation-models":
                stage_env.update(DAKIA_REAL_MODELS="1", DAKIA_MODEL_CACHE=str(ROOT / "target/release-translation-models"))
            process = subprocess.Popen(stage.command, cwd=cwd, env=stage_env, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=stage.timeout)
                result.update(exit_code=code, status="passed" if code == 0 else "failed")
            except subprocess.TimeoutExpired:
                stop_tree(process)
                result.update(status="timed_out", reason=f"exceeded {stage.timeout}s")
            except BaseException:
                stop_tree(process)
                raise
        except OSError as error:
            result.update(status="blocked", reason=str(error))
            stream.write(str(error) + "\n")
    result["elapsed_seconds"] = round(time.monotonic() - started, 2)
    # Cargo's stable text summary, including ignored tests, is retained as data.
    summaries = re.findall(r"test result: .*? (\d+) passed; (\d+) failed; (\d+) ignored;", log.read_text(errors="replace"))
    if summaries:
        result["tests"] = dict(zip(("passed", "failed", "ignored"), [sum(int(row[i]) for row in summaries) for i in range(3)]))
    return result


def run(stages_to_run: list[Stage], output: Path, env: dict[str, str], executor=execute) -> list[dict]:
    results = []
    for stage in stages_to_run:
        blocked_by = [name for name in stage.requires if not any(r["name"] == name and r["status"] == "passed" for r in results)]
        if blocked_by:
            result = {"name": stage.name, "status": "blocked", "reason": "prerequisite failed: " + ", ".join(blocked_by)}
        else:
            print(f"Starting {stage.name}", flush=True)
            result = executor(stage, output, env)
        results.append(result)
        print(f"{stage.name}: {result['status']}", flush=True)
        (output / "stages.json").write_text(json.dumps(results, indent=2) + "\n")
    return results


def preflight(profile: str, env: dict[str, str], dovecot_ssh: str | None = None) -> list[str]:
    problems = []
    for command in ("cargo", "node", "npm"):
        if not shutil.which(command, path=env.get("PATH")):
            problems.append(f"{command} is missing from PATH")
    if shutil.which("node", path=env.get("PATH")):
        version = subprocess.check_output(["node", "--version"], env=env, text=True).strip()
        if not re.match(r"v(22|24)\.", version):
            problems.append(f"Use Node 22 or 24 for jsdom tests; found {version}")
    for path in ("node_modules/.bin/vitest", "node_modules/.bin/playwright", "workers/push/node_modules/web-push", "workers/translation/node_modules/parse5"):
        if not (ROOT / path).exists():
            problems.append(f"Install locked dependencies: missing {path}")
    assets = ROOT / "crates/inference/assets/email-classifier-v2"
    model = assets / "model.onnx"
    if not model.exists():
        problems.append("Run python3 scripts/fetch-classifier.py before testing")
    else:
        expected = json.loads((assets / "MANIFEST.json").read_text())["files"][model.name]
        with model.open("rb") as stream:
            if hashlib.file_digest(stream, "sha256").hexdigest() != expected:
                problems.append("Classifier checksum differs from the pinned manifest")
    if profile == "full":
        if not env.get("CHROME_PATH") or not Path(env["CHROME_PATH"]).is_file():
            problems.append("Set CHROME_PATH to Google Chrome's executable")
        try:
            command = ["ssh", "-o", "BatchMode=yes", dovecot_ssh, "docker info"] if dovecot_ssh else ["docker", "info"]
            subprocess.run(command, env=env, check=True, capture_output=True, timeout=15)
        except (OSError, subprocess.SubprocessError):
            problems.append("Docker is unavailable; real Dovecot acceptance is blocked")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", choices=("core", "full"), default="full")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--dovecot-ssh", help="Run only the disposable Dovecot container on this SSH host")
    args = parser.parse_args()
    if args.dovecot_ssh and args.dovecot_ssh.startswith("-"):
        parser.error("--dovecot-ssh requires a host, not an SSH option")
    os.chdir(ROOT)
    output = (args.output or ROOT / "docs/verification" / datetime.now(timezone.utc).strftime("release-%Y%m%dT%H%M%SZ")).resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, TZ="Europe/Tallinn", CI="1")
    # Explicit toolchain selection defeats unrelated parent-shell overrides.
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    env["RUSTUP_TOOLCHAIN"] = channel
    if not env.get("CHROME_PATH"):
        for chrome in ("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome", "/usr/bin/google-chrome-stable"):
            if Path(chrome).is_file(): env["CHROME_PATH"] = chrome; break
    # Test-only loopback fixtures must not inherit operator/provider secrets.
    for name in list(env):
        if name.startswith("DAKIA_"): env.pop(name)
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    status = subprocess.check_output(["git", "status", "--short"], text=True)
    report = {"head": head, "working_tree": status, "profile": args.profile, "output": str(output), "release_ready": False}
    manifest = json.dumps(source_manifest(), sort_keys=True, indent=2) + "\n"
    (output / "source-manifest.json").write_text(manifest)
    report["source_manifest_sha256"] = hashlib.sha256(manifest.encode()).hexdigest()
    problems = preflight(args.profile, env, args.dovecot_ssh)
    report["preflight"] = problems
    # Preflight failures are recorded, but independent stages still run. A
    # missing browser or Docker must not suppress storage or API diagnostics.
    for problem in problems: print(f"Preflight: {problem}", flush=True)
    results = run(stages(args.profile, channel, args.dovecot_ssh), output, env)
    report["stages"] = results
    report["release_ready"] = args.profile == "full" and not problems and all(r["status"] == "passed" for r in results)
    (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"Report: {output / 'summary.json'}", flush=True)
    return 0 if not problems and all(r["status"] == "passed" for r in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
