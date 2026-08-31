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
