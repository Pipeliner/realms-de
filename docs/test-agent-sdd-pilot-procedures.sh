#!/bin/sh
set -eu

root=$(CDPATH='' cd "$(dirname "$0")/.." && pwd)
skills=$root/.claude/skills

require_text() {
    file=$1
    text=$2
    label=$3
    grep -F -q -e "$text" "$file" || {
        echo "FAIL: $label" >&2
        exit 1
    }
}

require_absent() {
    file=$1
    text=$2
    label=$3
    if grep -F -q -e "$text" "$file"; then
        echo "FAIL: $label" >&2
        exit 1
    fi
}

require_front_matter() {
    file=$1
    skill=$2
    [ "$(sed -n '1p' "$file")" = '---' ] || {
        echo "FAIL: $skill front matter start" >&2
        exit 1
    }
    [ "$(sed -n '2p' "$file")" = "name: $skill" ] || {
        echo "FAIL: $skill front matter name" >&2
        exit 1
    }
    description=$(sed -n '3p' "$file")
    case "$description" in
        'description: "'*'"') ;;
        *)
            echo "FAIL: $skill front matter description" >&2
            exit 1
            ;;
    esac
    description=${description#description:\ \"}
    description=${description%\"}
    [ -n "$description" ] &&
        ! printf '%s\n' "$description" | grep -F -q -e '"' -e '\\' || {
        echo "FAIL: $skill front matter description scalar" >&2
        exit 1
    }
    [ "$(sed -n '4p' "$file")" = '---' ] || {
        echo "FAIL: $skill front matter end" >&2
        exit 1
    }
}

require_command_surface() {
    file=$1
    skill=$2
    if ! sed -n 's/^[[:space:]]*//p' "$file" | awk '
        /helm-sdd / {
            if ($0 != "helm-sdd gate --issue <issue> --from <maturity> --to <maturity>" &&
                $0 != "helm-sdd promote --dry-run --issue <issue> --from <maturity> --to <maturity>") {
                invalid = 1
            }
            count += 1
        }
        END { exit(invalid || count != 2 ? 1 : 0) }
    '; then
        echo "FAIL: $skill command surface" >&2
        exit 1
    fi
}

for skill in helm-agent-sdd-bootstrap helm-agent-sdd-checkpoint helm-agent-sdd-evidence-capture; do
    file=$skills/$skill/SKILL.md
    [ -f "$file" ] || {
        echo "FAIL: missing $skill" >&2
        exit 1
    }
    require_front_matter "$file" "$skill"
    require_text "$file" 'SPEC 0008' "$skill governance reference"
    require_text "$file" '## Do not use when' "$skill negative trigger guidance"
    require_text "$file" 'helm-sdd gate' "$skill read only gate"
    require_text "$file" 'promote --dry-run' "$skill dry run assessment"
    require_command_surface "$file" "$skill"
    require_text "$file" 'Pilot exclusions: no hook, daemon, scheduler, CI job, service, external database, embedding search or third-party integration.' "$skill excluded automation"
done

checkpoint=$skills/helm-agent-sdd-checkpoint/SKILL.md
evidence=$skills/helm-agent-sdd-evidence-capture/SKILL.md
require_text "$checkpoint" '.agent/work/<issue>/checkpoint.toml' 'checkpoint file contract'
require_text "$checkpoint" '.agent/work/<issue>/evidence.jsonl' 'evidence file contract'
require_text "$checkpoint" 'record-carrier commit' 'fresh record carrier contract'
require_text "$checkpoint" 'does not promote' 'report only contract'
require_text "$evidence" 'command output' 'command output exclusion'
require_text "$evidence" 'secrets' 'secret exclusion'
require_text "$evidence" 'absolute paths' 'absolute path exclusion'
require_text "$evidence" 'source snapshots' 'source snapshot exclusion'

require_text "$skills/README.md" '[`helm-agent-sdd-bootstrap`](helm-agent-sdd-bootstrap/)' 'bootstrap skill index'
require_text "$skills/README.md" '[`helm-agent-sdd-checkpoint`](helm-agent-sdd-checkpoint/)' 'checkpoint skill index'
require_text "$skills/README.md" '[`helm-agent-sdd-evidence-capture`](helm-agent-sdd-evidence-capture/)' 'evidence skill index'
require_text "$root/.claude/memory/40-loop.md" 'Pilot procedure measurement' 'pilot measurement loop guidance'

echo 'PASS: agent SDD pilot procedures'
