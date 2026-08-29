#!/bin/sh

set -eu

usage() {
    echo "usage: $0 --date YYYY-MM-DD" >&2
    exit 2
}

fail() {
    echo "Fedora baseline invalid: $*" >&2
    exit 1
}

decimal() {
    value=$1
    while [ "${value#0}" != "$value" ]; do
        value=${value#0}
    done
    printf '%s\n' "${value:-0}"
}

valid_date() {
    value=$1
    case "$value" in
        [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]) ;;
        *) return 1 ;;
    esac

    year=${value%%-*}
    remainder=${value#*-}
    month=${remainder%%-*}
    day=${remainder#*-}

    year_number=$(decimal "$year")
    month_number=$(decimal "$month")
    day_number=$(decimal "$day")

    [ "$year_number" -ge 1 ] || return 1
    case "$month_number" in
        1 | 3 | 5 | 7 | 8 | 10 | 12) last_day=31 ;;
        4 | 6 | 9 | 11) last_day=30 ;;
        2)
            if [ $((year_number % 400)) -eq 0 ] || {
                [ $((year_number % 4)) -eq 0 ] &&
                    [ $((year_number % 100)) -ne 0 ]
            }; then
                last_day=29
            else
                last_day=28
            fi
            ;;
        *) return 1 ;;
    esac

    [ "$day_number" -ge 1 ] && [ "$day_number" -le "$last_day" ]
}

[ "$#" -eq 2 ] || usage
[ "$1" = "--date" ] || usage
evaluation_date=$2
valid_date "$evaluation_date" || fail "evaluation date is not a real YYYY-MM-DD date: $evaluation_date"

script_dir=$(CDPATH='' cd "$(dirname "$0")" && pwd)
manifest="$script_dir/baseline.toml"
[ -r "$manifest" ] || fail "cannot read $manifest"

schema=
status=
release=
eol=
image=
schema_seen=false
status_seen=false
release_seen=false
eol_seen=false
image_seen=false
line_number=0

while IFS= read -r line || [ -n "$line" ]; do
    line_number=$((line_number + 1))
    case "$line" in
        'schema = "'*'"')
            [ "$schema_seen" = false ] || fail "duplicate schema field on line $line_number"
            schema=${line#schema = \"}
            schema=${schema%\"}
            schema_seen=true
            ;;
        'status = "'*'"')
            [ "$status_seen" = false ] || fail "duplicate status field on line $line_number"
            status=${line#status = \"}
            status=${status%\"}
            status_seen=true
            ;;
        'release = '*)
            [ "$release_seen" = false ] || fail "duplicate release field on line $line_number"
            release=${line#release = }
            release_seen=true
            ;;
        'eol = "'*'"')
            [ "$eol_seen" = false ] || fail "duplicate eol field on line $line_number"
            eol=${line#eol = \"}
            eol=${eol%\"}
            eol_seen=true
            ;;
        'image = "'*'"')
            [ "$image_seen" = false ] || fail "duplicate image field on line $line_number"
            image=${line#image = \"}
            image=${image%\"}
            image_seen=true
            ;;
        '') ;;
        *) fail "unknown or malformed field on line $line_number" ;;
    esac
done <"$manifest"

[ "$schema_seen" = true ] || fail "missing schema field"
[ "$status_seen" = true ] || fail "missing status field"
[ "$release_seen" = true ] || fail "missing release field"
[ "$eol_seen" = true ] || fail "missing eol field"
[ "$image_seen" = true ] || fail "missing image field"

[ "$schema" = "helm-fedora-baseline/v1" ] || fail "unsupported schema: $schema"
case "$status" in
    pre-alpha | unsupported) ;;
    *) fail "unsupported status: $status" ;;
esac
valid_date "$eol" || fail "EOL is not a real YYYY-MM-DD date: $eol"

expected_image='registry.fedoraproject.org/fedora:44@sha256:df52038ff64ee61affa188d78beb85cf6eecfe4e9f6042238269ccdc8e944392'
[ "$release" = 44 ] || fail "SPEC 0009 admits only Fedora release 44"
[ "$eol" = 2027-06-02 ] || fail "Fedora 44 EOL does not match SPEC 0009"
[ "$image" = "$expected_image" ] || fail "Fedora 44 image does not match SPEC 0009"

if [ "$status" = unsupported ]; then
    echo "Fedora baseline is explicitly unsupported; lifecycle comparison skipped"
    exit 0
fi

evaluation_number=$(printf '%s' "$evaluation_date" | tr -d '-')
eol_number=$(printf '%s' "$eol" | tr -d '-')
[ "$evaluation_number" -lt "$eol_number" ] ||
    fail "Fedora 44 reached its recorded EOL on $eol"

echo "Fedora 44 pre-alpha baseline is within lifecycle through the day before $eol"
