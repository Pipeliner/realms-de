#!/usr/bin/env python3
"""Exercise the actual runtime fixture's receipt expression without packages."""
import ast
from pathlib import Path
import re
from types import SimpleNamespace
import unittest


class ReceiptTests(unittest.TestCase):
    def parse(self, receipt):
        tree = ast.parse(Path(__file__).with_name("test-tool-runtime.py").read_text())
        assignments = [node for node in ast.walk(tree)
                       if isinstance(node, ast.Assign)
                       and any(isinstance(target, ast.Name) and target.id == "match"
                               for target in node.targets)]
        self.assertEqual(len(assignments), 1)
        expression = ast.Expression(assignments[0].value)
        return eval(compile(expression, "runtime-receipt", "eval"),
                    {"re": re, "apply": SimpleNamespace(stdout=receipt)})

    def test_current_receipt_selects_exact_generation(self):
        match = self.parse("generation 0123456789abcdef0123456789abcdef prepared for next graphical login\n")
        self.assertIsNotNone(match)
        self.assertEqual(match.group(1), "0123456789abcdef0123456789abcdef")

    def test_rejects_legacy_malformed_or_extra_output(self):
        for receipt in (
            "generation 0123456789abcdef0123456789abcdef selected for future launches\n",
            "generation ../escape prepared for next graphical login\n",
            "generation 0123456789abcdef prepared for next graphical login\n",
            "generation 0123456789ABCDEF0123456789ABCDEF prepared for next graphical login\n",
            "generation 0123456789abcdef0123456789abcdef prepared for next graphical login",
            "generation 0123456789abcdef0123456789abcdef prepared for next graphical login\nextra\n",
        ):
            with self.subTest(receipt=receipt):
                self.assertIsNone(self.parse(receipt))


if __name__ == "__main__":
    unittest.main()
