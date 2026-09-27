#!/usr/bin/env python3
"""The suspend source checks consume evaluated data without child processes."""
from pathlib import Path
import runpy
import sys


def reject_child_process(event, arguments):
    if event == "subprocess.Popen":
        raise AssertionError("suspend source test attempted a builder child process")


sys.addaudithook(reject_child_process)
runpy.run_path(str(Path(__file__).with_name("test_suspend_vm_helpers.py")))
