#!/bin/sh

set -eu

expected='/usr/lib/rust-1.[89][0-9]/bin'
root=/

if [ "$#" -eq 2 ] && [ "$1" = "--root" ]; then
  root=$2
elif [ "$#" -ne 0 ]; then
  printf '%s\n' "usage: $0 [--root ROOT]" >&2
  exit 2
fi

case $root in
  /) prefix= ;;
  *) prefix=${root%/} ;;
esac

selected=
for candidate in "$prefix"/usr/lib/rust-1.[89][0-9]/bin; do
  [ -x "$candidate/cargo" ] && [ -x "$candidate/rustc" ] || continue
  selected=${candidate#"$prefix"}
done

if [ -z "$selected" ]; then
  printf '%s\n' "helm: missing Ubuntu versioned Rust toolchain directory: $expected" >&2
  exit 1
fi

printf '%s\n' "$selected"
