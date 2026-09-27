#!/usr/bin/env python3
"""Exercise the real login entry with its desktop side effects replaced."""
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest


class LoginClaim(unittest.TestCase):
    def test_real_compositor_and_supervisor_do_not_inherit_login_claim(self):
        source = Path(__file__).with_name("realm-session").read_text()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            child = root / "inspect-child"
            child.write_text('''#!/usr/bin/env python3
import os
from pathlib import Path
root = Path(os.environ["XDG_RUNTIME_DIR"])
lock = root / "realm-session.lock"
inherited = any(p.resolve() == lock for p in Path("/proc/self/fd").iterdir())
(root / os.environ["RESULT"]).write_text("inherited" if inherited else "closed")
raise SystemExit(78)
''')
            child.chmod(0o700)
            entry = root / "entry"
            entry.write_text(source.removesuffix('main "$@"\n') + r'''
log() { :; }
claim_login
export RESULT=compositor-result
start_compositor
wait "$compositor_pid" || [[ $? == 78 ]]
export RESULT=supervisor-result
spawn_supervised "$REALM_COMPOSITOR" 0
wait "${direct_pids[0]}"
# Children closing their copy must not release the entry's own claim.
flock --exclusive --nonblock "$XDG_RUNTIME_DIR/realm-session.lock" true && exit 99
exit 0
''')
            entry.chmod(0o700)
            result = subprocess.run([str(entry)], capture_output=True, text=True,
                                    timeout=5, env=dict(os.environ,
                                        XDG_RUNTIME_DIR=str(root),
                                        REALM_COMPOSITOR=str(child)))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((root / "compositor-result").read_text(), "closed")
            self.assertEqual((root / "supervisor-result").read_text(), "closed")

    def test_competing_login_cannot_touch_active_session_and_relogin_succeeds(self):
        source = Path(__file__).with_name("realm-session").read_text()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "realm-session.lock").touch(mode=0o600)
            entry = root / "entry"
            # Retain the real main ordering and claim; stop before compositor work.
            entry.write_text(source.removesuffix('main "$@"\n') + r'''
setup_log() { :; }
set_identity() { :; }
ensure_session_bus() { :; }
detect_session_services() { :; }
start_compositor() {
    [[ $(<"$XDG_RUNTIME_DIR/theme-args") == $'--prepare-session-theme\n'"$$" ]] || exit 98
    printf '%s\n' "$$" > "$XDG_RUNTIME_DIR/touched"
    trap 'printf "teardown\n" > "$XDG_RUNTIME_DIR/teardown"; exit 0' TERM HUP
    read -r release
    exit 0
}
main "$@"
''')
            entry.chmod(0o700)
            helper = root / "realm-wm"
            helper.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$XDG_RUNTIME_DIR/theme-args"\n')
            helper.chmod(0o700)
            env = dict(os.environ, XDG_RUNTIME_DIR=str(root), REALM_WM=str(helper))
            first = subprocess.Popen([str(entry)], env=env, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                deadline = time.monotonic() + 5
                while not (root / "touched").exists() and first.poll() is None:
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(0.01)
                self.assertIsNone(first.poll(), first.communicate()[1] if first.poll() is not None else "")
                original = (root / "touched").read_text()
                second = subprocess.run([str(entry)], env=env, input="release\n",
                                        capture_output=True, text=True, timeout=5)
                self.assertEqual(second.returncode, 73, second.stderr)
                self.assertEqual((root / "touched").read_text(), original)
                first.terminate()
                first.communicate(timeout=5)
                self.assertEqual(first.returncode, 0)
                self.assertEqual((root / "teardown").read_text(), "teardown\n")
                again = subprocess.run([str(entry)], env=env, input="release\n",
                                       capture_output=True, text=True, timeout=5)
                self.assertEqual(again.returncode, 0, again.stderr)
                self.assertNotEqual((root / "touched").read_text(), original)
            finally:
                if first.poll() is None:
                    first.communicate("release\n", timeout=5)


if __name__ == "__main__":
    unittest.main()
