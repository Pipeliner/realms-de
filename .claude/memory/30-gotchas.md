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
