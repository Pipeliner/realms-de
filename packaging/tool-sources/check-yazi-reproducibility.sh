#!/bin/sh
# CI-only: repeat the production Yazi recipe, then compare declared artifacts.
# Caller supplies a fresh retained Debian kit and the same isolated toolchain
# environment as its initial successful Debian build.
set -eu
if [ "$#" -ne 3 ]; then
    echo "usage: $0 FIRST_KIT SECOND_FRESH_KIT EVIDENCE_FILE" >&2
    exit 2
fi
first=$(CDPATH='' cd "$1" && pwd -P)
second=$(CDPATH='' cd "$2" && pwd -P)
evidence=$3
case $evidence in
    /*) ;;
    *) evidence=$(pwd)/$evidence ;;
esac
if [ "$first" = "$second" ] || [ -e "$second/debian/yazi-target" ] \
    || [ -e "$second/debian/yazi-25.4.8" ]; then
    echo 'Yazi reproducibility requires a separate clean second kit' >&2
    exit 1
fi
for name in yazi ya; do
    test -f "$first/debian/yazi-target/release/$name"
done
{
    printf 'first=%s\nsecond=%s\nnormalization=none\n' "$first" "$second"
    printf 'repeat-start-utc=%s\n' "$(date -u +%FT%TZ)"
    printf 'recipe=make -f debian/rules realm-build-yazi\n'
} > "$evidence"
if (cd "$second" && make -f debian/rules realm-build-yazi) >> "$evidence" 2>&1; then
    printf 'repeat-end-utc=%s\n' "$(date -u +%FT%TZ)" >> "$evidence"
else
    printf 'result=build-failed\n' >> "$evidence"
    exit 1
fi
different=0
for name in yazi ya; do
    first_binary=$first/debian/yazi-target/release/$name
    second_binary=$second/debian/yazi-target/release/$name
    sha256sum "$first_binary" "$second_binary" >> "$evidence"
    if ! cmp "$first_binary" "$second_binary" >> "$evidence" 2>&1; then
        printf 'result=different:%s\n' "$name" >> "$evidence"
        different=1
    fi
done
if [ "$different" -ne 0 ]; then
    echo "Yazi reproducibility mismatch; see $evidence" >&2
    tail -n 40 "$evidence" >&2
    exit 1
fi
printf 'result=identical\n' >> "$evidence"
