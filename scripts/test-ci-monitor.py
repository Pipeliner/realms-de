"""Exercise read-only monitoring against a controlled GitHub response."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


MONITOR = Path(__file__).resolve().with_name("ci-monitor")


class MonitorTests(unittest.TestCase):
    def invoke(self, scenario, mode="--once", settings=None):
        with tempfile.TemporaryDirectory(prefix="realm-monitor-test-") as temporary:
            root = Path(temporary)
            fake = root / "gh"
            fake.write_text("""#!/usr/bin/env python3
import json, os, sys, urllib.parse, time
if sys.argv[1:3] == ['repo', 'view']:
    print('owner/repo')
    sys.exit(0)
if os.environ['MONITOR_CASE'] == 'error':
    print('upstream unavailable', file=sys.stderr)
    sys.exit(7)
if os.environ['MONITOR_CASE'] == 'malformed':
    print('{}')
    sys.exit(0)
if os.environ['MONITOR_CASE'] == 'slow':
    time.sleep(5)
if sys.argv[1] == 'api' and 'branch=main' in sys.argv[2]:
    status = urllib.parse.parse_qs(sys.argv[2].split('?', 1)[1]).get('status', [''])[0]
    case = os.environ['MONITOR_CASE']
    wanted = 'in_progress' if case in ('old-active', 'paginated', 'incomplete', 'rerun') else case
    runs = [{'id': 123, 'name': 'ci',
        'status': 'completed', 'conclusion': 'success', 'head_branch': 'main',
        'head_sha': 'a' * 40, 'updated_at': '2026-09-27T00:00:00Z'}] if not status else []
    if not status and case == 'rerun':
        runs[0].update(id=99, name='distro')
    if not status and case == 'large-response':
        runs[0]['repository_metadata'] = 'x' * 200000
    if status and status == wanted:
        runs = [{'id': 99, 'name': 'distro', 'status': status, 'conclusion': None,
                 'head_branch': 'main', 'head_sha': 'b' * 40, 'updated_at': '2026-09-26T00:00:00Z'}]
        if case == 'paginated':
            print(json.dumps({'total_count': 1, 'workflow_runs': []}))
            if '--paginate' not in sys.argv:
                sys.exit(0)
    print(json.dumps({'total_count': 1001 if case == 'incomplete' and status == wanted else len(runs), 'workflow_runs': runs}))
else:
    # All thirty newest repository-wide runs are PRs; older main still exists.
    print(json.dumps([{'databaseId': i, 'name': 'ci', 'headBranch': 'feature',
        'status': 'completed', 'conclusion': 'success'} for i in range(30)]))
""")
            fake.chmod(0o755)
            environment = dict(os.environ, PATH=str(root) + os.pathsep + os.environ["PATH"],
                               MONITOR_CASE=scenario, CI_MONITOR_INTERVAL_SECONDS="1",
                               CI_MONITOR_MAX_SECONDS="2")
            environment.update(settings or {})
            return subprocess.run(
                ["bash", str(MONITOR), mode], capture_output=True, text=True,
                timeout=5,
                env=environment,
            )

    def test_recent_pr_traffic_cannot_hide_main(self):
        result = self.invoke("traffic")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "123\tci\tcompleted\tsuccess\t" + "a" * 40
                         + "\t2026-09-27T00:00:00Z\n")

    def test_invalid_timing_settings_are_rejected(self):
        for variable in ("CI_MONITOR_INTERVAL_SECONDS", "CI_MONITOR_MAX_SECONDS"):
            for value in ("0", "no", "1.5", "-1"):
                with self.subTest(variable=variable, value=value):
                    result = self.invoke("traffic", settings={variable: value})
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("positive", result.stderr)

    def test_full_api_metadata_is_not_passed_through_argument_limit(self):
        result = self.invoke('large-response')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('123\tci\tcompleted', result.stdout)

    def test_failed_or_malformed_queries_do_not_report_success(self):
        for scenario in ("error", "malformed"):
            with self.subTest(scenario=scenario):
                result = self.invoke(scenario)
                self.assertNotEqual(result.returncode, 0)

    def test_repeated_errors_stop_at_watch_budget(self):
        result = self.invoke("error", "--watch")
        self.assertNotEqual(result.returncode, 0)

    def test_old_active_and_later_pages_cannot_age_out(self):
        for scenario in ('old-active', 'paginated', 'rerun'):
            with self.subTest(scenario=scenario):
                result = self.invoke(scenario)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn('99\tdistro\tin_progress\t', result.stdout)
                self.assertEqual(result.stdout.count('99\t'), 1)

    def test_nonterminal_statuses_never_report_watch_complete(self):
        for status in ('pending', 'waiting', 'requested', 'queued', 'in_progress'):
            with self.subTest(status=status):
                result = self.invoke(status, '--watch')
                self.assertEqual(result.returncode, 124, result.stderr)
                self.assertIn('99\tdistro\t' + status + '\t', result.stdout)

    def test_hung_query_and_incomplete_history_are_not_quiescence(self):
        for scenario in ('slow', 'incomplete'):
            with self.subTest(scenario=scenario):
                result = self.invoke(scenario, '--watch')
                self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
