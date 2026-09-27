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


if __name__ == '__main__':
    unittest.main()
