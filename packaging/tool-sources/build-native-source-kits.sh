#!/bin/sh
# Construct retained-only Debian and RPM source inputs without consulting Git.
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: build-native-source-kits.sh DESTINATION" >&2
    exit 1
fi
root=$(CDPATH='' cd "$(dirname "$0")/../.." && pwd)
destination=$1
mkdir -p "$destination"
destination=$(CDPATH='' cd "$destination" && pwd)
debian_output=$destination/realm-debian-0.1.0
rpm_output=$destination/realm-0.1.0.tar.gz
spec_output=$destination/realm.spec
for output in "$debian_output" "$rpm_output" "$spec_output"; do
    if [ -e "$output" ] || [ -L "$output" ]; then
        echo "native source-kit output already exists: $output" >&2
        exit 1
    fi
done

temporary=$(mktemp -d "$destination/.realm-native-source-kits.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
debian=$temporary/realm-debian-0.1.0
rpm_parent=$temporary/rpm
rpm=$rpm_parent/realm-0.1.0

copy_authority() {
    kit=$1
    mkdir -p "$kit/packaging/tool-sources/bundles"
    for helper in check-bundle-linkage.py check-native-source-kit.py stage-realm-workspace.py; do
        cp "$root/packaging/tool-sources/$helper" "$kit/packaging/tool-sources/$helper"
    done
    cp -R "$root/packaging/tool-sources/bundles/realm-workspace" \
        "$kit/packaging/tool-sources/bundles/realm-workspace"
}

copy_package_docs() {
    kit=$1
    mkdir -p "$kit/packaging/package-docs"
    cp "$root/docs/INSTALL.md" "$kit/packaging/package-docs/INSTALL.md"
}

mkdir -p "$debian/debian" "$rpm/packaging"
cp -R "$root/packaging/debian/." "$debian/debian/"
copy_authority "$debian"
copy_package_docs "$debian"
cp -R "$root/packaging/fedora" "$rpm/packaging/fedora"
copy_authority "$rpm"
copy_package_docs "$rpm"

"$root/packaging/tool-sources/check-native-source-kit.py" debian "$debian"
"$root/packaging/tool-sources/check-native-source-kit.py" rpm "$rpm"
tar --sort=name --mtime='@0' --owner=0 --group=0 --numeric-owner \
    -C "$rpm_parent" -czf "$temporary/realm-0.1.0.tar.gz" realm-0.1.0
cp "$root/packaging/fedora/realm.spec" "$temporary/realm.spec"

mv "$debian" "$debian_output"
mv "$temporary/realm-0.1.0.tar.gz" "$rpm_output"
mv "$temporary/realm.spec" "$spec_output"
