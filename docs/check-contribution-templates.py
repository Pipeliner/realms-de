#!/usr/bin/env python3
"""Validate the typed GitHub contribution forms required by SPEC 0017."""
import argparse
from pathlib import Path
import sys
import yaml

def fail(message):
    print(f"FAIL: {message}", file=sys.stderr)
    raise SystemExit(1)

def form(path):
    try:
        value = yaml.safe_load(path.read_text())
    except yaml.YAMLError as exc:
        fail(f"invalid YAML in {path.name}: {exc}")
    if not isinstance(value, dict) or not isinstance(value.get("body"), list):
        fail(f"{path.name} must be an issue form with a body list")
    return value

def fields(value):
    result = {}
    for item in value["body"]:
        if not isinstance(item, dict) or item.get("type") == "markdown":
            continue
        field_id = item.get("id")
        attrs = item.get("attributes")
        if not isinstance(field_id, str) or not isinstance(attrs, dict):
            fail("every contributor-answer item must have an id and attributes")
        if field_id in result:
            fail(f"duplicate field id: {field_id}")
        result[field_id] = item
    return result

def required(field_id, item, label):
    if item.get("type") not in {"input", "textarea"} or item["attributes"].get("label") != label:
        fail(f"field {field_id} must be a {label!r} contributor prompt")
    if item.get("validations", {}).get("required") is not True:
        fail(f"field {field_id} must be required")

parser = argparse.ArgumentParser()
parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
root = parser.parse_args().root
forms = root / ".github" / "ISSUE_TEMPLATE"

work = fields(form(forms / "work-item.yml"))
for name, label in {"why": "Why this matters", "acceptance": "Acceptance criteria", "blocked-by": "Blocked by:", "source": "Source:"}.items():
    if name not in work: fail(f"work-item missing {name}")
    required(name, work[name], label)
if "Given …, when …, then …" not in work["acceptance"]["attributes"].get("value", ""):
    fail("work-item acceptance must prefill Given/When/Then")

human = fields(form(forms / "needs-human.yml"))
if set(human) != {"decision", "options", "recommendation", "blocked"}:
    fail("needs-human must contain exactly decision, options, recommendation, and blocked")
for name, label in {"decision": "Decision needed", "options": "Options and honest trade-offs", "recommendation": "Recommendation", "blocked": "What is blocked until answered"}.items():
    required(name, human[name], label)

pr = (root / ".github" / "PULL_REQUEST_TEMPLATE.md").read_text()
for text in ("Closes #", "- Spec:", "cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test"):
    if text not in pr: fail(f"pull-request template missing {text!r}")
bug = fields(form(forms / "bug.yml"))
if "missed-guard" not in bug:
    fail("bug form missing missed-guard")
required("missed-guard", bug["missed-guard"], "Which guard from `docs/PITFALLS.md` should have caught it?")
print("contribution templates: pass")
