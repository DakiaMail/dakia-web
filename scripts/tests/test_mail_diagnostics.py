import importlib.util, json, pathlib, tempfile, unittest

spec = importlib.util.spec_from_file_location(
    "diagnostics", pathlib.Path(__file__).parents[1] / "mail-diagnostics.py"
)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


class Diagnostics(unittest.TestCase):
    def test_export_redacts_and_preserves_context(self):
        a = "a" * 64
        b = "b" * 64
        event = m.parse(
            f'2026-11-04T12:00:00Z INFO task{{account_key="{a}" purpose="search_index"}}: fetch{{request_key="{b}"}}: dakia_server::provider_io: imap_wire command="uid_fetch" response_bytes=42 elapsed_ms=2 completed=true private="someone@example.invalid" password="secret"'
        )
        self.assertEqual(event["account_key"], a)
        self.assertEqual(event["request_key"], b)
        self.assertEqual(event["purpose"], "search_index")
        self.assertTrue(event["completed"])
        self.assertNotIn("secret", json.dumps(event))
        self.assertNotIn("@", json.dumps(event))

    def test_fetch_recovery_events_remain_separate_from_wire_commands(self):
        account = "a" * 64
        eof = m.parse(f'2026-11-04T12:00:00Z INFO fetch{{account_key="{account}"}}: provider_io operation="body_size_check" advertised_bytes=100 body_bytes=7 outcome="eof_verified"')
        stale = m.parse(f'2026-11-04T12:00:01Z INFO raw{{account_key="{account}"}}: mail_work operation="stale_reference_reconcile" outcome="scheduled"')
        self.assertEqual(eof["advertised_bytes"], 100)
        self.assertEqual(eof["body_bytes"], 7)
        summary = m.analyze([eof, stale], {"coverage_complete": True})["accounts"][account]
        self.assertEqual(summary["body_size_checks"], {"eof_verified": 1})
        self.assertEqual(summary["stale_reference_reconciliations"], 1)
        self.assertEqual(summary["wire_commands"], {})
        self.assertEqual(summary["operation_events"], {})

    def test_reference_gaps_and_recovery_are_not_extra_wire_calls(self):
        a="a"*64
        ref="b"*64
        blocked=m.parse(f'2026-11-05T07:00:00Z INFO task{{account_key="{a}"}}: mail_work operation="content_reference_blocked" reference_key="{ref}"')
        gap=dict(event="mail_work",account_key=a,operation="search_index",outcome="reference_unavailable")
        recovered=dict(event="mail_work",account_key=a,operation="content_reference_recovery",outcome="resolved")
        summary=m.analyze([blocked,gap,recovered],{"coverage_complete":True})["accounts"][a]
        self.assertEqual(blocked["reference_key"],ref)
        self.assertEqual(summary["content_references_blocked"],1)
        self.assertEqual(summary["reference_unavailable_results"],1)
        self.assertEqual(summary["reference_recovery_results"],{"resolved":1})
        self.assertEqual(summary["wire_commands"],{})

    def test_real_window_filters_deduplicates_and_marks_gaps(self):
        with tempfile.TemporaryDirectory() as d:
            p = pathlib.Path(d) / "diagnostics"
            p.mkdir()
            base = 1793793600
            lines = [
                f'{m.utc(base+i).replace("+00:00","Z")} INFO x: mail_heartbeat schema_version=2\n'
                for i in range(0, 301, 60)
            ]
            (p / "mail-events.0.log").write_text("".join(lines))
            (p / "mail-events.1.log").write_text(lines[0])
            events, w = m.collect(d, base, base + 300)
            self.assertTrue(w["coverage_complete"])
            self.assertEqual(len(events), 6)
            (p / "mail-events.0.log").write_text(lines[0] + lines[-1])
            events, w = m.collect(d, base, base + 300)
            self.assertFalse(w["coverage_complete"])

    def test_analysis_counts_wire_only_and_detects_permanent_repeats(self):
        a = "a" * 64
        events = []
        for i in range(2):
            events.append(
                dict(
                    time=i,
                    event="mail_work",
                    account_key=a,
                    message_key="b" * 64,
                    source_key="c" * 64,
                    operation="search_index",
                    outcome="oversized",
                )
            )
        events += [
            dict(
                event="imap_wire",
                account_key=a,
                command="uid_fetch",
                completed=True,
                response_bytes=5,
                purpose="search_index",
            ),
            dict(event="provider_io", account_key=a, operation="reconcile_budget"),
            dict(event="mail provider cache lane", account_key=a, operation="fetch"),
        ]
        result = m.analyze(events, {"coverage_complete": True})
        s = result["accounts"][a]
        self.assertEqual(s["wire_commands"], {"uid_fetch": 1})
        self.assertEqual(s["repeated_permanent_index_results"], 1)
        self.assertNotIn("reconcile_budget", s["operation_events"])

    def test_partial_lines_and_missing_directory_are_not_clean_windows(self):
        with tempfile.TemporaryDirectory() as d:
            _, w = m.collect(d, 0, 100)
            self.assertFalse(w["coverage_complete"])
            p = pathlib.Path(d) / "diagnostics"
            p.mkdir()
            (p / "mail-events.0.log").write_text(
                "2026-11-04T12:00:00Z INFO x: mail_heartbeat"
            )
            _, w = m.collect(d, 0, 100)
            self.assertFalse(w["coverage_complete"])
            self.assertEqual(w["partial_lines"], 1)

    def test_byte_totals_exclude_legacy_buffers_and_incomplete_measurements(self):
        events = [
            dict(
                event="imap_wire",
                account_key="a" * 64,
                command="uid_fetch",
                completed=True,
                response_bytes=8192,
            ),
            dict(
                event="imap_wire",
                account_key="a" * 64,
                command="uid_fetch",
                completed=True,
                wire_schema=2,
                response_bytes=17,
                response_bytes_complete=True,
            ),
            dict(
                event="imap_wire",
                account_key="a" * 64,
                command="uid_fetch",
                completed=False,
                wire_schema=2,
                response_bytes=5,
                response_bytes_complete=False,
            ),
        ]
        summary = m.analyze(events, {"coverage_complete": True})["accounts"]["a" * 64]
        self.assertEqual(summary["wire_response_bytes"], 17)
        self.assertEqual(summary["wire_unmeasured_responses"], 2)


class ExportFiles(unittest.TestCase):
    def test_private_artifact_output_and_reanalysis(self):
        with tempfile.TemporaryDirectory() as d:
            out = pathlib.Path(d) / "window"
            window = {
                "start": "2026-11-04T00:00:00Z",
                "end": "2026-11-04T06:00:00Z",
                "coverage_complete": False,
            }
            m.write_output(out, [], window)
            self.assertEqual(out.stat().st_mode & 0o777, 0o700)
            self.assertEqual((out / "events.jsonl").stat().st_mode & 0o777, 0o600)
            self.assertIn("incomplete", (out / "report.md").read_text())
            with self.assertRaises(FileExistsError):
                m.write_output(out, [], window)

    def test_symlinks_are_rejected_instead_of_exporting_unrelated_files(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "diagnostics").mkdir()
            secret = root / "unrelated"
            secret.write_text("private@example.invalid")
            (root / "diagnostics" / "mail-events.0.log").symlink_to(secret)
            events, w = m.collect(d, 0, 100)
            self.assertEqual(events, [])
            self.assertFalse(w["coverage_complete"])
            self.assertIn("segment_open_failed", w["collector_errors"])


if __name__ == "__main__":
    unittest.main()
