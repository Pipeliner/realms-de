# River protocol source provenance

These files are unmodified copies of the five Realm MVP protocol XML files in
the upstream River repository:

- Upstream: <https://codeberg.org/river/river>
- Release: `v0.4.8`
- Annotated tag object: `b028abcff33ed7a42ee222dce1abe71c06d3e41a`
- Source commit: `c4b5f706314555f4846e25b8d3635631387b3fdd`
- Source commit timestamp: `2026-08-07T12:51:29+02:00`
- Source paths: `protocol/<filename>`
- License: MIT, as declared and reproduced in each XML's copyright element
- Upstream license copy: `LICENSES/MIT.txt`, preserved here at the same path

The source revision and tag were revalidated against the live upstream
repository on 2026-09-12. The retained bytes have these SHA-256 digests:

| File | SHA-256 |
|---|---|
| `river-window-management-v1.xml` | `71ea28c64e48522bd818df46dbe7d76d62ccdd55ddbe7e3421ef261bdf980225` |
| `river-layer-shell-v1.xml` | `b25919ad9bb60e4fd1937d9bb69729dca726d16bd9e32472e7434afabfe39fa1` |
| `river-xkb-bindings-v1.xml` | `5ac514a178395f69391857f10f8486369baf9b435663a8cd582994154715fd95` |
| `river-input-management-v1.xml` | `122ada3beb5d4cb5984fe122f7886c2a048e261852a50e8d2e6926e3958bb98b` |
| `river-libinput-config-v1.xml` | `b548aac4c9c19c61146c50ce589e05c26e476cc0c6053227f72d197dc70b0108` |
| `LICENSES/MIT.txt` | `b05785f9f18e6716bab63424b11454513b9943a222595b70411009202fc592b5` |

The XML remains the authoritative generated-binding input. Do not normalize,
reformat, or hand-edit it. A River upgrade must replace the bytes from one
reviewed upstream revision, update this record, and rerun the interface-version
contract test.
