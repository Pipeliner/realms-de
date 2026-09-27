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
diagnose_binary() {
    printf 'elf-diagnostics:%s:%s\n' "$1" "$name"
    if command -v readelf >/dev/null 2>&1; then
        readelf --wide --file-header --section-headers --notes "$2" 2>&1 | head -n 160
        for section in .text .rodata .data .debug_info .debug_str; do
            printf 'section-content-sha256:%s\n' "$section"
            readelf --hex-dump="$section" "$2" 2>&1 | sha256sum
        done
    else
        printf 'readelf unavailable\n'
    fi
    printf 'embedded-build-paths:%s:%s\n' "$1" "$name"
    if command -v strings >/dev/null 2>&1; then
        strings -a "$2" | awk -v first="$first" -v second="$second" \
            'index($0, first) || index($0, second) { print substr($0, 1, 512); if (++n == 20) exit }'
    else
        printf 'strings unavailable\n'
    fi
}
for tool in rustc cc ld readelf; do
    printf 'diagnostic-tool-version:%s\n' "$tool" >> "$evidence"
    if command -v "$tool" >/dev/null 2>&1; then
        "$tool" --version 2>&1 | head -n 5 >> "$evidence"
    else
        printf 'unavailable\n' >> "$evidence"
    fi
done
for name in yazi ya; do
    first_binary=$first/debian/yazi-target/release/$name
    second_binary=$second/debian/yazi-target/release/$name
    sha256sum "$first_binary" "$second_binary" >> "$evidence"
    if ! cmp "$first_binary" "$second_binary" >> "$evidence" 2>&1; then
        {
            diagnose_binary first "$first_binary"
            diagnose_binary second "$second_binary"
            sha256sum "$first_binary" "$second_binary"
            printf 'result=different:%s\n' "$name"
        } >> "$evidence" 2>&1
        different=1
    fi
done
if [ "$different" -ne 0 ]; then
    echo "Yazi reproducibility mismatch; see $evidence" >&2
    tail -n 40 "$evidence" >&2
    exit 1
fi
printf 'result=identical\n' >> "$evidence"
