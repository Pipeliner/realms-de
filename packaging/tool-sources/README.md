# Retained tool-source intake

This directory is the offline, reviewed source-intake record for SPEC 0023.
`check-intake.py` validates the source archive bytes, notices, and complete
inventory. It does **not** make Yazi or Starship available to Debian or Fedora:
those targets require retained Cargo dependency closures, native package
integration, availability evidence, and rollback tests before they can be
claimed as supported.

`test-no-live-fetch.sh` is a narrow regression guard against obvious live
source-acquisition commands in the current native package definitions. It is
not evidence for the complete A2 offline-build requirement.

`bundles/helm-workspace/` is the immutable source authority for the Helm
workspace's own Cargo invocations.  Its `source.tar.gz`, lockfile, Cargo
source-replacement configuration, compressed vendor tree, license report, and
provenance record are all digest-bound in `bundle.toml`.  The source archive is
created once by controlled intake from the recorded repository commit; package
recipes must stage and build that archive rather than make another archive or
use a moving checkout.
