#!/usr/bin/env python3
"""Export and analyze private, content-free Dakia diagnostics. No provider calls.

Local: mail-diagnostics.py --data-dir /data --out NEW_DIR
Remote: mail-diagnostics.py --ssh mail-host --data-dir /opt/dakia/data/app --out NEW_DIR
Analysis: mail-diagnostics.py --input DIR/events.jsonl --out NEW_DIR
"""
import argparse
import collections
import datetime as dt
import hashlib
import json
import os
import pathlib
import re
import shlex
import subprocess
import time

ANSI = re.compile(r"\x1b\[[0-9;]*m")
STAMP = re.compile(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z")
FIELD = re.compile(r'\b([a-z_]+)=(?:"([^"\r\n]*)"|([A-Za-z0-9_-]+))')
HASHES = {
    "account_key",
    "message_key",
    "source_key",
    "folder_key",
    "request_key",
    "reference_key",
    "operation_key",
    "job_key",
}
NUMBERS = {
    "rows",
    "header_bytes",
    "mutable_rows",
    "unchanged_rows",
    "elapsed_ms",
    "response_bytes",
    "wire_schema",
    "indexed_bytes",
    "advertised_bytes",
    "body_bytes",
    "schema_version",
    "max_files",
    "max_file_bytes",
    "wait_ms",
    "deadline_ms",
    "lane_wait_ms",
    "checkout_ms",
    "batch",
}
BOOLS = {
    "completed",
    "response_bytes_complete",
    "complete",
    "has_next",
    "unchanged",
    "reused",
    "delta",
    "acquired",
    "write_started",
    "write_acknowledged",
}
WORDS = {
    "operation",
    "purpose",
    "command",
    "outcome",
    "reason",
    "stage",
    "kind",
    "provider",
    "request_kind",
}
EVENTS = (
    "model job attempt failed",
    "mail mutation claimed",
    "mail action deferred before provider write",
    "mail diagnostic retention active",
    "inbox refresh completed; saved mail retained",
    "inbox refresh completed",
    "inbox refresh started",
    "mail provider request failed",
    "mail mutation not verified",
    "mail mutation timing",
    "mail provider cache lane",
    "mail_work",
    "imap_wire",
    "imap_connection",
    "mail_heartbeat",
    "provider_io",
    "mail diagnostic retention write failed",
)
AUXILIARY = {
    "connection_attempt",
    "connection_backoff",
    "provider_backoff",
    "reconcile_budget",
    "storage_wait",
    "idle_wait",
    "mutation_timing",
    "scan_result",
    "delta_fetch_skipped",
    "body_size_check",
}


def utc(t):
    return dt.datetime.fromtimestamp(t, dt.timezone.utc).isoformat()


def parse(line):
    line = ANSI.sub("", line)
    stamp = STAMP.match(line)
    if not stamp:
        return None
    event = next((e for e in EVENTS if e in line), None)
    if not event:
        return None
    result = {
        "time": dt.datetime.fromisoformat(stamp[0].replace("Z", "+00:00")).timestamp(),
        "event": event,
    }
    if re.search(r"\s(?:WARN|ERROR)\s", line):
        result["warning"] = True
    for match in FIELD.finditer(line):
        key = match[1]
        value = match[2] if match[2] is not None else match[3]
        if key in HASHES and re.fullmatch("[a-f0-9]{64}", value):
            result[key] = value
        elif key in NUMBERS and value.isdecimal():
            result[key] = int(value)
        elif key in BOOLS and value in ("true", "false"):
            result[key] = value == "true"
        elif key in WORDS and re.fullmatch("[A-Za-z_]{1,64}", value):
            result[key] = value
    return result


def collect(directory, start, end):
    """Pin open segments before reading to tolerate ordinary ring rotation."""
    directory = pathlib.Path(directory) / "diagnostics"
    handles = []
    seen_inodes = set()
    errors = []
    for path in sorted(directory.glob("mail-events.*.log"), reverse=True):
        if not re.fullmatch(r"mail-events\.\d{1,2}\.log", path.name):
            continue
        if int(path.name.split(".")[1]) >= 32:
            errors.append("unexpected_segment")
            continue
        try:
            fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
            stat = os.fstat(fd)
            if stat.st_size > 8 * 1024 * 1024:
                errors.append("segment_size_exceeded")
            if (stat.st_dev, stat.st_ino) in seen_inodes:
                os.close(fd)
                continue
            seen_inodes.add((stat.st_dev, stat.st_ino))
            handles.append((os.fdopen(fd, "rb"), min(stat.st_size, 8 * 1024 * 1024)))
        except OSError:
            errors.append("segment_open_failed")
    events = []
    seen = set()
    heartbeats = []
    partial_lines = 0
    read_bytes = 0
    try:
        for stream, size in handles:
            raw = stream.read(size)
            read_bytes += len(raw)
            if raw and not raw.endswith(b"\n"):
                partial_lines += 1
                raw = raw.rsplit(b"\n", 1)[0] if b"\n" in raw else b""
            for line in raw.decode("utf-8", errors="replace").splitlines():
                stamp = STAMP.match(ANSI.sub("", line))
                if not stamp:
                    continue
                when = dt.datetime.fromisoformat(
                    stamp[0].replace("Z", "+00:00")
                ).timestamp()
                if not start - 150 <= when <= end:
                    continue
                event = parse(line)
                if not event:
                    continue
                if event["event"] == "mail_heartbeat":
                    heartbeats.append(when)
                if when < start:
                    continue
                key = json.dumps(event, sort_keys=True)
                if key not in seen:
                    events.append(event)
                    seen.add(key)
    finally:
        for stream, _ in handles:
            stream.close()
    events.sort(key=lambda e: e["time"])
    heartbeats = sorted(set(heartbeats))
    gaps = [b - a for a, b in zip(heartbeats, heartbeats[1:])]
    covered = bool(
        heartbeats
        and heartbeats[0] <= start + 60
        and heartbeats[-1] >= end - 120
        and max(gaps, default=0) <= 150
        and not errors
        and not partial_lines
    )
    metadata = {
        "type": "window",
        "start": utc(start),
        "end": utc(end),
        "hours": (end - start) / 3600,
        "coverage_complete": covered,
        "heartbeat_count": len(heartbeats),
        "largest_heartbeat_gap_seconds": max(gaps, default=None),
        "segment_count": len(handles),
        "bytes_read": read_bytes,
        "partial_lines": partial_lines,
        "collector_errors": errors,
        "events": len(events),
    }
    return events, metadata


def analyze(events, window):
    by_account = collections.defaultdict(list)
    for event in events:
        by_account[event.get("account_key", "unattributed")].append(event)
    result = {"window": window, "accounts": {}, "issues": []}
    if not window.get("coverage_complete"):
        result["issues"].append(
            {"kind": "incomplete_log_coverage", "severity": "warning"}
        )
    for account, rows in sorted(by_account.items()):
        wire = [e for e in rows if e["event"] == "imap_wire"]
        work = [e for e in rows if e["event"] == "mail_work"]
        pages = [
            e
            for e in rows
            if e["event"] == "provider_io" and e.get("operation") == "reconcile"
        ]
        failures = [e for e in rows if e["event"] == "mail provider request failed"]
        connections = [e for e in rows if e["event"] == "imap_connection"]
        permanent = collections.defaultdict(list)
        for e in work:
            if (
                e.get("operation") == "search_index"
                and e.get("outcome")
                in ("oversized", "encrypted", "parse_error", "quota")
                and e.get("message_key")
                and e.get("source_key")
            ):
                permanent[(e["message_key"], e["source_key"], e["outcome"])].append(e)
        repeats = sum(max(0, len(es) - 1) for es in permanent.values())
        summary = {
            "wire_commands": dict(
                collections.Counter(e.get("command", "other") for e in wire)
            ),
            "wire_by_purpose": dict(
                collections.Counter(e.get("purpose", "unattributed") for e in wire)
            ),
            "wire_by_request": dict(
                collections.Counter(e.get("request_kind", "unattributed") for e in wire)
            ),
            "wire_incomplete": sum(not e.get("completed", False) for e in wire),
            "wire_response_bytes": sum(
                e.get("response_bytes", 0)
                for e in wire
                if e.get("wire_schema") == 2 and e.get("response_bytes_complete")
            ),
            "wire_unmeasured_responses": sum(
                e.get("wire_schema") != 2 or not e.get("response_bytes_complete")
                for e in wire
            ),
            "operation_events": dict(
                collections.Counter(
                    e["operation"]
                    for e in rows
                    if e["event"] == "provider_io"
                    and e.get("operation") not in AUXILIARY
                    and "operation" in e
                )
            ),
            "failure_reasons": dict(
                collections.Counter(e.get("reason", "unknown") for e in failures)
            ),
            "model_failures": sum(
                e["event"] == "model job attempt failed" for e in rows
            ),
            "connection_attempt_outcomes": dict(collections.Counter(
                e.get("outcome", "unknown") for e in rows
                if e.get("event") == "provider_io" and e.get("operation") == "connection_attempt"
            )),
            "connections_suppressed": sum(
                e.get("event") == "provider_io" and e.get("operation") == "connection_backoff"
                for e in rows
            ),
            "new_connections": sum(e.get("reused") is False for e in connections),
            "reused_connections": sum(e.get("reused") is True for e in connections),
            "reconciliation_rows": sum(e.get("rows", 0) for e in pages),
            "unchanged_rows": sum(e.get("unchanged_rows", 0) for e in pages),
            "delta_fetches_skipped": sum(
                e.get("operation") == "delta_fetch_skipped"
                for e in rows
                if e["event"] == "provider_io"
            ),
            "starred_scan_results": sum(
                e.get("operation") == "scan_result" and e.get("purpose") == "starred"
                for e in rows
            ),
            "empty_starred_scan_results": sum(
                e.get("operation") == "scan_result"
                and e.get("purpose") == "starred"
                and e.get("rows") == 0
                for e in rows
            ),
            "repeated_permanent_index_results": repeats,
            "search_index_results": dict(
                collections.Counter(
                    e.get("outcome", "unknown")
                    for e in work
                    if e.get("operation") == "search_index"
                )
            ),
            "starred_scans_skipped": sum(
                e.get("operation") == "starred_scan" and e.get("outcome") == "skipped"
                for e in work
            ),
            "body_size_checks": dict(
                collections.Counter(
                    e.get("outcome", "unknown")
                    for e in rows
                    if e.get("operation") == "body_size_check"
                )
            ),
            "content_references_blocked": sum(e.get("operation")=="content_reference_blocked" for e in work),
            "reference_unavailable_results": sum(e.get("operation")=="search_index" and e.get("outcome")=="reference_unavailable" for e in work),
            "reference_recovery_results": dict(collections.Counter(e.get("outcome","unknown") for e in work if e.get("operation")=="content_reference_recovery")),
            "stale_reference_reconciliations": sum(
                e.get("operation") == "stale_reference_reconcile"
                and e.get("outcome") == "scheduled"
                for e in work
            ),
        }
        result["accounts"][account] = summary
        for kind, count in [
            ("repeated_permanent_index_results", repeats),
            ("provider_failures", len(failures)),
            ("incomplete_wire_commands", summary["wire_incomplete"]),
            ("model_failures", summary["model_failures"]),
        ]:
            if count:
                result["issues"].append(
                    {
                        "kind": kind,
                        "account_key": account,
                        "count": count,
                        "severity": "warning",
                    }
                )
        if (
            summary["reconciliation_rows"] >= 1000
            and summary["unchanged_rows"] / summary["reconciliation_rows"] > 0.99
        ):
            result["issues"].append(
                {
                    "kind": "mostly_unchanged_reconciliation",
                    "account_key": account,
                    "severity": "candidate",
                    "note": "Review coverage and cursor progress before classifying as waste.",
                }
            )
    return result


def report(result):
    w = result["window"]
    lines = [
        "# Dakia mail diagnostics",
        f"\nWindow: {w['start']} to {w['end']}.",
        f"\nCoverage: {'complete' if w.get('coverage_complete') else 'incomplete or not yet verified'}. Missing coverage must not be interpreted as no failures.",
        "\n| Account key | Wire commands | Provider failures | Permanent result repeats | Starred skips |",
        "| --- | ---: | ---: | ---: | ---: |",
    ]
    for account, s in result["accounts"].items():
        lines.append(
            f"| {account} | {sum(s['wire_commands'].values())} | {sum(s['failure_reasons'].values())} | {s['repeated_permanent_index_results']} | {s['starred_scans_skipped']} |"
        )
    lines += ["\n## Findings", ""]
    lines += [
        f"- {i['kind']}: {i.get('count','review')} ({i.get('account_key','window')}). {i.get('note','')}"
        for i in result["issues"]
    ] or ["No configured issue detectors fired."]
    lines += [
        "\nWire commands exclude login, TLS negotiation, SMTP and the long-lived IDLE protocol. Provider-operation boundaries and wire commands are separate counts. Incomplete wire commands can include cancellation or uncertain writes and must never trigger automatic mutation replay. High unchanged ratios are optimization candidates, not proof of wasted work. No provider probes were added."
    ]
    return "\n".join(lines) + "\n"


def write_output(out, events, window):
    os.umask(0o077)
    out.mkdir(mode=0o700, parents=True, exist_ok=False)
    with (out / "events.jsonl").open("w") as f:
        for e in events:
            f.write(json.dumps(e, separators=(",", ":")) + "\n")
    (out / "window.json").write_text(json.dumps(window, indent=2) + "\n")
    result = analyze(events, window)
    (out / "analysis.json").write_text(json.dumps(result, indent=2) + "\n")
    (out / "report.md").write_text(report(result))
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in out.iterdir()}
    (out / "sha256.json").write_text(json.dumps(hashes, indent=2) + "\n")
    print(
        json.dumps(
            {
                "report": str(out / "report.md"),
                "coverage_complete": window.get("coverage_complete"),
                "issues": len(result["issues"]),
            }
        )
    )


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--data-dir", default="/opt/dakia/data/app")
    p.add_argument("--hours", type=float, default=6)
    p.add_argument("--out", type=pathlib.Path)
    p.add_argument("--stdout", action="store_true")
    p.add_argument("--ssh")
    p.add_argument("--input", type=pathlib.Path)
    p.add_argument(
        "--end", help="Timezone-aware ISO timestamp, for a reproducible past window"
    )
    args = p.parse_args()
    if not 0 < args.hours <= 72:
        p.error("--hours must be in (0,72]")
    if not args.stdout and args.out is None:
        p.error("--out is required")
    if args.ssh:
        if args.ssh.startswith("-"):
            p.error("invalid SSH host")
        remote_args = [
            "python3",
            "-",
            "--data-dir",
            args.data_dir,
            "--hours",
            str(args.hours),
            "--stdout",
        ]
        if args.end:
            remote_args += ["--end", args.end]
        command = shlex.join(remote_args)
        response = subprocess.run(
            ["ssh", "-o", "BatchMode=yes", args.ssh, command],
            input=pathlib.Path(__file__).read_text(),
            text=True,
            capture_output=True,
            check=True,
            timeout=180,
        )
        data = [json.loads(s) for s in response.stdout.splitlines()]
        windows = [x for x in data if x.get("type") == "window"]
        if len(windows) != 1:
            raise RuntimeError("missing unique window metadata")
        events = [x["event"] for x in data if x.get("type") == "event"]
        window = windows[0]
    elif args.input:
        events = [json.loads(s) for s in args.input.read_text().splitlines()]
        window = json.loads(args.input.with_name("window.json").read_text())
    else:
        end = time.time()
        if args.end:
            end_at = dt.datetime.fromisoformat(args.end.replace("Z", "+00:00"))
            if end_at.tzinfo is None:
                p.error("--end must include a timezone")
            end = end_at.timestamp()
            if end > time.time() + 1:
                p.error("--end cannot be in the future")
        events, window = collect(args.data_dir, end - args.hours * 3600, end)
    if args.stdout:
        for e in events:
            print(json.dumps({"type": "event", "event": e}, separators=(",", ":")))
        print(json.dumps(window, separators=(",", ":")))
    else:
        write_output(args.out, events, window)


if __name__ == "__main__":
    main()
