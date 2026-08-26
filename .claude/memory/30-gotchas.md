# Gotchas

Things that cost time once and must not cost it twice. Each entry names the
symptom first, because that is what you will be searching for.

---

**Symptom:** `cannot serialize tagged newtype variant X containing an integer`.
**Cause:** serde's *internally* tagged enums (`#[serde(tag = "…")]`) cannot
represent newtype variants wrapping non-map values.
**Fix:** use adjacent tagging — `#[serde(tag = "cmd", content = "arg")]`.

**Symptom:** a hue-preservation test fails on a near-grey like `#c2cbde`.
**Cause:** hue angle is numerically meaningless at low chroma; the value is
noise.
**Fix:** gate hue assertions on `chroma() > 0.02` and test greys for lightness
instead.

**Symptom:** an accent turns pale and shifts hue at high contrast.
**Cause:** OKLab lightness pushed past the sRGB gamut boundary, then clamped
per-channel.
**Fix:** cap lightness at the boundary (`reachable_lightness`) so chroma and hue
both survive. Never clamp channels on a colour whose hue matters.

**Symptom:** `cargo` refuses to build the workspace — "failed to load manifest
for workspace member".
**Cause:** a `members` entry for a crate directory that does not exist yet.
**Fix:** add crates to `members` when they gain a manifest, not when they are
planned.

**Symptom:** `cd` inside a `Bash` tool call silently reverts for the next call.
**Cause:** each invocation starts from the project root; `cd` does not persist.
**Fix:** use absolute paths in scripts, or `cd` within the same compound command.

**Symptom:** a river mapping table that looks too good, with every row "exact".
**Cause:** reading a protocol summary rather than its XML. `place_*` orders the
*render* list, not layout order; `propose_dimensions` is a proposal a client may
quantise; and three companion protocols are obligations the summary omits.
**Fix:** for any protocol helm depends on, read the XML from the source
repository before writing a mapping table. A second reader who checks the
primary source is worth more than a careful first reader who does not.

**Symptom:** a dependency described as "unstable" on the strength of a tracking
issue.
**Cause:** the issue predated the release that stabilised it.
**Fix:** check the release announcement and the protocol file itself (a `z`
prefix, an `unstable/` directory, the interface version) before repeating a
stability claim. `river-window-management-v1` is declared stable as of 0.4.0.
