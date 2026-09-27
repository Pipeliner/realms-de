#!/bin/sh
set -eu
root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
. "$root/packaging/tool-sources/native-version-check.sh"
starship_version_is_selected 'starship 1.23.0'
starship_version_is_selected 'starship 1.23.0
branch:
commit_hash:
build_time:2026-08-26 00:00:00 +00:00
build_env:rustc 1.98.1,'
for invalid in '' 'starship 1.23.00' 'starship 1.24.0' 'starship 1.23.0 unexpected' 'warning
starship 1.23.0'; do
    if starship_version_is_selected "$invalid"; then
        echo 'FAIL: accepted wrong version identity' >&2
        exit 1
    fi
done
echo 'PASS: Starship first-line version identity'
