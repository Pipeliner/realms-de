#!/bin/sh

set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
resolver="$repo_root/packaging/debian/toolchain-path.sh"
rules="$repo_root/packaging/debian/rules"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

fail() {
  printf '%s\n' "FAIL: $*" >&2
  exit 1
}

pass() {
  printf '%s\n' "PASS: $1"
}

make_toolchain() {
  version=$1
  mkdir -p "$tmp/root/usr/lib/rust-$version/bin"
  : > "$tmp/root/usr/lib/rust-$version/bin/cargo"
  : > "$tmp/root/usr/lib/rust-$version/bin/rustc"
  chmod +x "$tmp/root/usr/lib/rust-$version/bin/cargo" \
    "$tmp/root/usr/lib/rust-$version/bin/rustc"
}

make_toolchain 1.85
make_toolchain 1.90

selected=$("$resolver" --root "$tmp/root") || fail "newest-complete-toolchain resolver failed"
[ "$selected" = "/usr/lib/rust-1.90/bin" ] \
  || fail "newest-complete-toolchain selected $selected"
pass newest-complete-toolchain

mkdir -p "$tmp/incomplete/usr/lib/rust-1.85/bin"
: > "$tmp/incomplete/usr/lib/rust-1.85/bin/cargo"
chmod +x "$tmp/incomplete/usr/lib/rust-1.85/bin/cargo"
if output=$("$resolver" --root "$tmp/incomplete" 2>&1); then
  fail "incomplete-toolchain unexpectedly succeeded: $output"
fi
printf '%s\n' "$output" | grep -F "/usr/lib/rust-1.[89][0-9]/bin" >/dev/null \
  || fail "incomplete-toolchain diagnostic did not name expected path: $output"
pass incomplete-toolchain

mkdir -p "$tmp/empty"
if output=$("$resolver" --root "$tmp/empty" 2>&1); then
  fail "missing-toolchain unexpectedly succeeded: $output"
fi
printf '%s\n' "$output" | grep -F "/usr/lib/rust-1.[89][0-9]/bin" >/dev/null \
  || fail "missing-toolchain diagnostic did not name expected path: $output"
pass missing-toolchain

# The native retained-source fixture cannot rely on the host's Ubuntu package
# database, but it must still exercise the same resolver (including Debhelper's
# nested make).  A supplied resolver root is therefore a test seam, not a
# RUST_VERSIONED_BIN bypass.
if output=$(make -f "$rules" -pn \
  HELM_RUST_VERSIONED_ROOT="$tmp/root" override_dh_auto_build 2>&1); then
  printf '%s\n' "$output" | grep -F \
    "RUST_VERSIONED_BIN := $tmp/root/usr/lib/rust-1.90/bin" >/dev/null \
    || fail "rules-configured-root did not select the physical supplied toolchain: $output"
else
  fail "rules-configured-root unexpectedly failed: $output"
fi
pass rules-configured-root

if output=$(make -f "$rules" -n RUST_VERSIONED_BIN= override_dh_auto_build 2>&1); then
  fail "rules-missing-toolchain unexpectedly succeeded: $output"
fi
printf '%s\n' "$output" | grep -F "/usr/lib/rust-1.[89][0-9]/bin" >/dev/null \
  || fail "rules-missing-toolchain diagnostic did not name expected path: $output"
pass rules-missing-toolchain
