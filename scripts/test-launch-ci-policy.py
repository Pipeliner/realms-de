#!/usr/bin/env python3
"""Check the event/job configuration supplied to GitHub, without building packages."""
from pathlib import Path
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[1]


def workflow(name):
    # BaseLoader preserves GitHub's `on` key instead of YAML 1.1 boolean coercion.
    return yaml.load((ROOT / '.github/workflows' / name).read_text(), Loader=yaml.BaseLoader)


class LaunchPolicy(unittest.TestCase):
    def test_rust_test_gate_is_bounded_and_retains_failure_diagnostics(self):
        job = workflow('ci.yml')['jobs']['test']
        self.assertEqual(job.get('timeout-minutes'), '25')
        test = next(s for s in job['steps'] if 'cargo test --workspace --all-features' in s.get('run', ''))
        self.assertEqual(test.get('timeout-minutes'), '20')
        self.assertEqual(test.get('shell'), 'bash')
        self.assertIn('set -o pipefail', test['run'])
        self.assertIn('cargo test --workspace --all-features 2>&1 | tee rust-tests.log', test['run'])
        self.assertNotIn('continue-on-error', test)
        upload = next(s for s in job['steps'] if s.get('name') == 'Retain Rust test diagnostics')
        self.assertEqual(upload['if'], 'always()')
        self.assertTrue(upload['uses'].startswith('actions/upload-artifact@'))
        self.assertEqual(upload['with']['path'], 'rust-tests.log')
        self.assertEqual(upload['with']['retention-days'], '7')

    def test_branch_push_cannot_duplicate_pr_workflows(self):
        for name in ('ci.yml', 'distro.yml', 'palette.yml'):
            with self.subTest(workflow=name):
                events = workflow(name)['on']
                self.assertEqual((events['push'] or {}).get('branches'), ['main'])
                self.assertIn('pull_request', events)
                self.assertIn('workflow_dispatch', events)

    def test_ready_transition_runs_full_checks_without_a_commit(self):
        for name in ('ci.yml', 'distro.yml'):
            with self.subTest(workflow=name):
                types = (workflow(name)['on']['pull_request'] or {}).get('types', [])
                self.assertTrue({'opened', 'synchronize', 'reopened', 'ready_for_review'} <= set(types))

    def test_only_pr_runs_can_cancel_running_verification(self):
        for name in ('ci.yml', 'distro.yml', 'palette.yml'):
            with self.subTest(workflow=name):
                self.assertEqual(workflow(name)['concurrency']['cancel-in-progress'],
                                 "${{ github.event_name == 'pull_request' }}")

    def test_packaging_jobs_require_candidate_or_non_pr_event(self):
        jobs = list(workflow('distro.yml')['jobs'].values())
        jobs.append(workflow('ci.yml')['jobs']['docs'])
        for job in jobs:
            with self.subTest(job=job.get('name')):
                self.assertEqual(job.get('if'),
                                 "github.event_name != 'pull_request' || !github.event.pull_request.draft")

    def test_routine_dependency_creation_paused(self):
        config = yaml.load((ROOT / '.github/dependabot.yml').read_text(), Loader=yaml.BaseLoader)
        self.assertEqual({u['package-ecosystem'] for u in config['updates']}, {'cargo', 'github-actions'})
        for update in config['updates']:
            self.assertEqual(update['open-pull-requests-limit'], '0')

    def test_failed_native_fixture_uploads_only_persistent_text_logs(self):
        steps = workflow('ci.yml')['jobs']['docs']['steps']
        fixture = next(s for s in steps if s.get('name') == 'Check network-isolated native package paths')
        self.assertIn('REALM_NATIVE_EVIDENCE_DIR=', fixture['run'])
        uploads = [s for s in steps if s.get('name') == 'Retain native fixture diagnostics']
        self.assertEqual(len(uploads), 1)
        upload = uploads[0]
        self.assertEqual(upload['if'], 'always()')
        self.assertEqual(upload['with']['path'].splitlines(), [
            '${{ steps.native-temp.outputs.path }}/evidence/*.out',
            '${{ steps.native-temp.outputs.path }}/evidence/*.log',
            '${{ steps.native-temp.outputs.path }}/evidence/yazi-reproducibility.txt',
        ])

    def test_native_fixture_has_bounded_cold_build_budget(self):
        job = workflow('ci.yml')['jobs']['docs']
        fixture = next(s for s in job['steps'] if s.get('name') == 'Check network-isolated native package paths')
        self.assertEqual(int(fixture['timeout-minutes']), 120)
        self.assertEqual(int(job['timeout-minutes']), 135)


if __name__ == '__main__':
    unittest.main()
