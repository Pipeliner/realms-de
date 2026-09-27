#!/bin/sh
# Run only on verified, unpacked build trees; never on retained archives.
set -eu
for staged_tree do
    find "$staged_tree" -type f -name '*.rs' -exec chmod a-x {} +
done
