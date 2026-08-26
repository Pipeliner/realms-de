# Decision log

A running index of calls made while working, and the reasoning that would
otherwise be lost. Full decisions live in `docs/adr/`; this file is the trail
between them, including the ones too small for an ADR.

Format: `date — decision — because — revisit if`.

---

**2026-08-26 — One crate in the workspace today, eight planned; members added
only when a crate has a real implementation** — because a fresh clone that does not build
is the worst first impression a repo can make — *revisit if* stub crates become
useful for CI wiring.

**2026-08-26 — `helm-core` written before anything else, and it owns every
shared type** — because two components inventing the same struct in parallel is
the single most expensive mistake in a multi-crate build, and subagents can only
be fanned out safely once the contract is committed — *revisit if* a client
needs a type the core should not know about (then it belongs to that client).

**2026-08-26 — ~~Ship on niri first~~ — superseded the same day. helm is instead
the *window manager* for a pinned river 0.4.x** — because river 0.4 removed
window management from the compositor and `river-window-management-v1` gives
exact positions, dimensions, node ordering, hide/show and borders, which is
helm's ledger model rather than an approximation of it. The niri route would
have shipped the signature triptych as an approximation with no stow at all —
*revisit if* the unstable protocol classification bites, or if vendoring river
proves untenable on Ubuntu or Fedora. See ADR 0013, and ADR 0002 for the
evidence that moved us.

**2026-08-26 — Vendor a pinned river rather than depending on the distro's** —
because Ubuntu 24.04 and Fedora 41 ship river 0.3.x or river-classic, neither of
which speaks the WM protocol, and waiting for them would break the MVP on two of
three first-class targets. Puts Zig in the packaging pipeline only — *revisit if*
the distros catch up.

**2026-08-26 — Contrast capped at the sRGB gamut boundary instead of
desaturating** — because pushing an accent past the boundary and clamping
channels rotates its hue; helm's violet drifted 18° toward blue at contrast 1.4
before this — *revisit if* we ever target a wide-gamut output, where the
boundary moves.

**2026-08-26 — Adjacently tagged serde enums (`tag` + `content`) for the IPC
protocol** — because internally tagged enums cannot serialise newtype variants
holding integers or sequences, which the protocol is full of — *revisit if* the
protocol moves to a binary encoding.

**2026-08-26 — Layout tunables (`TriptychParams`) are ratios, not pixels, and
default to the reference desktop's proportions** — because the mockup is 1920×1080
and the same shape has to survive 2560×1440 and 3840×2160 — *revisit if* users
want per-output pixel pinning.
