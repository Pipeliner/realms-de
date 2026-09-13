#!/usr/bin/env python3
import json
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from pathlib import Path

import xwayland_window_observation as observation


TITLE = "Realm X11 Probe A17-[4242].*"


class ObservationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tempdir = tempfile.TemporaryDirectory()
        self.addCleanup(self.tempdir.cleanup)
        self.root = Path(self.tempdir.name)
        self.timeout = shutil.which("timeout")
        assert self.timeout is not None

    def write_xwininfo(self, source: str) -> Path:
        executable = self.root / "xwininfo"
        executable.write_text(
            "#!/usr/bin/env python3\n" + textwrap.dedent(source),
            encoding="utf-8",
        )
        executable.chmod(0o755)
        return executable

    def observe(
        self,
        executable: Path,
        command_timeout: str = "1s",
    ) -> dict[str, object]:
        return observation.observe_window(
            timeout_bin=self.timeout,
            xwininfo_bin=str(executable),
            display=":77",
            title=TITLE,
            command_timeout=command_timeout,
        )

    def test_exact_unique_title_and_viewable_state_are_required(self) -> None:
        executable = self.write_xwininfo(
            f'''\
            import sys
            if "-root" in sys.argv:
                print('  0x40000a "{TITLE}": ("xmessage" "Xmessage") 184x52+0+0')
            else:
                print("Map State: IsViewable")
            '''
        )

        result = self.observe(executable)

        self.assertTrue(result["viewable"])
        self.assertEqual(result["window_ids"], ["0x40000a"])
        self.assertEqual(result["tree"]["status"], 0)
        self.assertEqual(result["stats"][0]["status"], 0)

    def test_duplicate_exact_titles_are_not_accepted(self) -> None:
        executable = self.write_xwininfo(
            f'''\
            print('  0x40000a "{TITLE}": first')
            print('  0x40000b "{TITLE}": second')
            '''
        )

        result = self.observe(executable)

        self.assertFalse(result["viewable"])
        self.assertEqual(result["window_ids"], ["0x40000a", "0x40000b"])
        self.assertEqual(result["stats"], [])

    def test_unmapped_window_is_not_accepted(self) -> None:
        executable = self.write_xwininfo(
            f'''\
            import sys
            if "-root" in sys.argv:
                print('  0x40000a "{TITLE}": exact')
            else:
                print("Map State: IsUnMapped")
            '''
        )

        result = self.observe(executable)

        self.assertFalse(result["viewable"])
        self.assertEqual(len(result["stats"]), 1)
        self.assertEqual(result["stats"][0]["status"], 0)

    def test_stalled_xwininfo_is_killed_by_the_bounded_executable(self) -> None:
        executable = self.write_xwininfo(
            '''\
            import time
            time.sleep(5)
            '''
        )
        started = time.monotonic()

        result = self.observe(executable, command_timeout="0.1s")

        self.assertLess(time.monotonic() - started, 2)
        self.assertEqual(result["tree"]["status"], 124)
        self.assertFalse(result["viewable"])

    def test_stalled_window_stats_has_the_same_command_bound(self) -> None:
        executable = self.write_xwininfo(
            f'''\
            import sys
            import time
            if "-root" in sys.argv:
                print('  0x40000a "{TITLE}": exact')
            else:
                time.sleep(5)
            '''
        )
        started = time.monotonic()

        result = self.observe(executable, command_timeout="0.1s")

        self.assertLess(time.monotonic() - started, 2)
        self.assertEqual(result["stats"][0]["status"], 124)
        self.assertFalse(result["viewable"])

    def test_cli_emits_the_complete_diagnostic_snapshot(self) -> None:
        executable = self.write_xwininfo(
            f'''\
            import sys
            if "-root" in sys.argv:
                print('  0x40000a "{TITLE}": exact')
            else:
                print("Map State: IsViewable")
            '''
        )
        completed = subprocess.run(
            [
                sys.executable,
                str(Path(observation.__file__)),
                "--timeout-bin",
                self.timeout,
                "--xwininfo-bin",
                str(executable),
                "--display",
                ":77",
                "--title",
                TITLE,
                "--command-timeout",
                "1s",
            ],
            check=False,
            capture_output=True,
            text=True,
            timeout=3,
        )

        self.assertEqual(completed.returncode, 0, completed.stderr)
        result = json.loads(completed.stdout)
        self.assertTrue(result["viewable"])
        self.assertIn("Map State: IsViewable", result["stats"][0]["output"])


if __name__ == "__main__":
    unittest.main()
