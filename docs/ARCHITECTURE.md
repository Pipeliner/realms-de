# helm — architecture

> **Status: provisional.** This document records the shape we are building
> towards and *why*. It is meant to be argued with. Every decision here has an
> ADR in [`docs/adr/`](adr/) with its alternatives and its reversal cost; a
> decision that turns out wrong gets a new ADR, not a quiet edit.

helm is a keyboard-first, gapless-tiling Wayland desktop environment. It is
Rust-first, it has no animations, and it treats window order as the only piece
of state that matters.

---

## 1. The one idea

Everything else follows from this:

```
                 user action
                      │
                      ▼
              ┌───────────────┐
              │    Ledger     │   ordered Vec<WinId> per orbit
              │  (the truth)  │   + undo history
              └───────┬───────┘
                      │  pure projection: fn(&Ledger, Layout, Workarea)
                      ▼
              ┌───────────────┐
              │  Vec<Placement>│  exact integer rects, no gaps, no overlap
              └───────┬───────┘
                      │  diff against last frame
                      ▼
               only what changed is redrawn
```

**Window positions are never stored.** They are recomputed from the ledger
whenever the ledger or the workarea changes. Three things fall out for free:

| Property | Why it follows |
|---|---|
| Undo is exact | Restore an older ledger; the screen is bit-identical. |
| Focus is free | Moving focus changes a flag, never a rectangle — verified by a test. |
| Relayout is skippable | Same inputs ⇒ same output ⇒ nothing to submit to the compositor. |

The projection lives in `helm-core::layout` and is already implemented and
tested against exact-tiling invariants at five resolutions and twelve window
counts.

---

## 2. Component map

```
                          ┌────────────────────────────────────────┐
   L0  contracts          │              helm-core                 │
                          │  ledger · layout · palette · color     │
                          │  keys · state · ipc · glyphs           │
                          └───────────────┬────────────────────────┘
                                          │ (every crate depends on this)
   ┌──────────────────────────────────────┼──────────────────────────────────┐
   │                                      │                                  │
   ▼                                      ▼                                  ▼
┌────────────────┐              ┌──────────────────┐              ┌────────────────┐
│  helm-session  │◀──unix sock──│    helm-ctl      │              │   helm-theme   │
│   (daemon)     │   NDJSON     │      (CLI)       │─────uses────▶│     (lib)      │
│                │              └──────────────────┘              │  palette.toml  │
│  owns HelmState│                                                │   → templates  │
│  WmBackend ────┼──▶ NiriBackend   (phase 1)                     └───────┬────────┘
│                │    NativeBackend (phase 3)                             │
└───────┬────────┘                                                        │ renders
        │ broadcasts HelmState                                            ▼
        │                                              gtk.css · kvantum · ANSI ·
        ├──────────────┬──────────────┐                yazi · btop · starship · fuzzel
        ▼              ▼              ▼
┌──────────────┐ ┌───────────┐ ┌────────────┐
│   helm-bar   │ │helm-hecate│ │  helm-odin │        L2  clients
│ layer-shell  │ │ launcher  │ │  ratatui   │
└──────────────┘ └───────────┘ └────────────┘

   L4  integration:  yazi (charon) · btop (horus) · zsh+starship (thoth)
                     xdg-desktop-portal · systemd user units · session entry
```

### Crate responsibilities

| Crate | Kind | Owns | Lands in |
|---|---|---|---|
| `helm-core` | lib | Ledger, layout projection, palette, keymap, IPC types, glyph inventory | **M0 — done** |
| `helm-theme` | lib | `palette.toml` → every themed file. Template engine, atomic writes, reload signalling | M1 |
| `helm-ctl` | bin | `helm ctl theme/orbit/ledger/doctor/run`. The scriptable surface | M1–M2 |
| `helm-session` | bin | Holds `HelmState`, drives a `WmBackend`, serves the control socket, launches clients | M2 |
| `helm-bar` | bin | Layer-shell bar, which-key strip, mode badge, chord echo | M2 |
| `helm-hecate` | bin | Layer-shell fuzzy launcher (`nucleo`) | M4 |
| `helm-odin` | bin | `ratatui` agent-harness TUI | M4 |
| `helm-compositor` | bin | Smithay compositor; `NativeBackend` for `helm-session` | M5 |

Crates join the Cargo workspace when they gain a real implementation, so a
fresh clone always builds.

---

## 3. Decisions

Each row links to its ADR. "Reversal" is the honest cost of changing our mind.

| # | Decision | Rationale in one line | Reversal |
|---|---|---|---|
| [0001](adr/0001-ledger-as-single-source-of-truth.md) | Ledger + pure projection | Undo, focus and damage-tracking all become trivial | Structural — the whole DE assumes it |
| [0002](adr/0002-borrow-a-compositor-first.md) | Ship on **niri** first, own compositor later | A compositor is 12+ months; the DE is the product. niri is Rust, on Smithay, and daily-drivable now | Low — hidden behind `WmBackend` |
| [0003](adr/0003-session-daemon-owns-state.md) | A session daemon owns state; clients subscribe | Bar/launcher stay dumb; swapping the compositor changes one file | Low |
| [0004](adr/0004-ndjson-control-socket.md) | Newline-delimited JSON over a unix socket | Scriptable with `socat`; partial frames can't be misread | Low |
| [0005](adr/0005-palette-toml-single-source.md) | One `palette.toml` → generated themes | No colour is written down twice; contrast is derived, not filtered | Low |
| [0006](adr/0006-oklab-contrast-not-filters.md) | Perceptual contrast derivation, gamut-capped | A `contrast()` filter costs a fullscreen pass and rotates hues | Low |
| [0007](adr/0007-reuse-yazi-btop-starship.md) | Reuse yazi / btop / zsh+starship rather than rewrite | charon and horus are ~90% theme + keymap. Rewrites cost years and lose features | Low |
| [0008](adr/0008-layer-shell-rendering-stack.md) | `smithay-client-toolkit` + `tiny-skia` + `cosmic-text` | Pure Rust, no GPU context for a 32px bar, real font fallback | Medium |
| [0009](adr/0009-no-animation-budget.md) | Zero animations; a frame budget, enforced in CI | "Snappy" is a number, not an adjective | Low |
| [0010](adr/0010-nix-flake-as-reference-build.md) | Nix flake is the reference build; deb/rpm generated | One reproducible definition; distro packages follow from it | Medium |
| [0011](adr/0011-session-integration-contract.md) | Session startup owns the D-Bus/systemd environment handshake | The #1 way minimal Wayland desktops break for users | Low |
| [0012](adr/0012-font-fallback-is-a-contract.md) | Glyph inventory + startup probe + ASCII fallbacks | A glyph-heavy DE must never ship tofu | Low |

### The compositor question, stated plainly

The brief asks for a Smithay compositor eventually. We agree — and we are not
starting there. `helm-session` talks to a `WmBackend` trait; `NiriBackend`
implements it against niri's IPC today, `NativeBackend` will implement it
in-process against `helm-compositor` later. Every client, every theme, every
keybinding and the entire ledger are written against the trait, not against
niri. Building the compositor first would mean a year before anyone can use the
thing, and the interesting design work — the ledger — is testable without it.

Marked **`needs-human`**: niri's own window model is scrollable-tiling, not
ledger-tiling. `NiriBackend` will therefore be a *projection*, not a passthrough,
and some helm layouts will not map perfectly onto it. See
[#adr-0002](adr/0002-borrow-a-compositor-first.md) for the mapping table and the
known gaps.

---

The seams named above — `WmBackend`, the template contract, the bar render
contract — are sketched with real signatures in
[docs/INTERFACES.md](INTERFACES.md), so M1 and M2 can proceed in parallel
without inventing the same types twice.

---

## 4. What "robust" and "snappy" mean here

Neither word is allowed to stay an adjective. Both are tests.

**Snappy — the frame budget** (ADR 0009):

| Path | Budget | How it is held |
|---|---|---|
| Key press → new geometry submitted | < 4 ms | Projection is pure integer maths; benchmarked in CI |
| State change → bar redraw | < 8 ms | Damage-tracked; `HelmState::renders_same_as` drops no-op frames |
| Bar idle CPU | ~0% | Event-driven modules; only the clock ticks, once a second |
| Cold session start → usable | < 900 ms | No GPU context for the bar, no icon-cache scan, no thumbnailer |
| `helm ctl theme apply` | < 150 ms | Templates rendered in parallel, written atomically |

**Robust — the failure modes we refuse to ship** (see
[docs/PITFALLS.md](PITFALLS.md) for the full register):

- Hairline cracks between tiles from rounding — *closed by exact-tiling tests.*
- Tofu glyphs on a machine without Nerd Fonts — *closed by the glyph probe.*
- Portals that hang because `WAYLAND_DISPLAY` never reached the D-Bus
  activation environment — *closed by the session contract (ADR 0011).*
- A theme change that half-applies and leaves the desktop two-toned — *closed by
  atomic writes plus a single reload fan-out.*
- A crashed bar taking the session with it — *clients are restartable units; the
  session daemon outlives them.*
- An unreadable palette after a contrast tweak — *closed by `palette lint` in CI.*

---

## 5. Target platforms

Supported from day one, tested in CI:

| Platform | Delivery | Notes |
|---|---|---|
| **NixOS / Nix** | Flake: packages, `nixosModules.helm`, `homeManagerModules.helm` | Reference build. A NixOS VM test boots the session and asserts the bar appears |
| **Ubuntu** 24.04 LTS + | `.deb` via `cargo-deb`, plus a PPA-shaped repo layout | Oldest supported glibc pins the MSRV story |
| **Fedora** 41 + | `.rpm` via `cargo-generate-rpm` and a `.spec` | SELinux-clean; no custom labels required |

Anything else is best-effort. The flake is the definition; the distro packages
are generated from the same metadata rather than maintained in parallel.

---

## 6. Repository layout

```
realms-de/
├─ crates/            # the Rust workspace (see §2)
├─ configs/           # shipped configs for reused tools
│  ├─ templates/      #   *.tmpl rendered from palette.toml
│  ├─ yazi/           #   charon: keymap + theme
│  ├─ btop/           #   horus: theme
│  ├─ zsh/ starship/  #   thoth: prompt
│  └─ portal/         #   xdg-desktop-portal wiring
├─ packaging/
│  ├─ nix/            # flake, module, VM test
│  ├─ debian/         # control, rules
│  └─ fedora/         # spec
├─ docs/
│  ├─ ARCHITECTURE.md # this file
│  ├─ MVP.md          # the cut line
│  ├─ PITFALLS.md     # the failure register
│  └─ adr/            # decisions, with alternatives and reversal costs
├─ design/            # the design handoff, for provenance
├─ .claude/           # operational memory, skills, the agentic loop
└─ palette.toml       # the one place a colour is written down
```

---

## 7. How this gets built

Work is tracked in GitHub milestones M0–M6; see [docs/MVP.md](MVP.md) for the
cut line and [docs/ROADMAP.md](ROADMAP.md) for what each milestone contains.
Issues that need a human judgement call — hardware access, a design trade-off,
a licence question — carry the **`needs-human`** label and are listed in the
README so they are never buried.
