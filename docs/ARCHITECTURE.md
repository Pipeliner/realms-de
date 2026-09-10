# realm — architecture

> **Status: ratified 2026-08-28.** This document records the shape we are building
> towards and *why*. It is meant to be argued with. Every decision here has an
> ADR in [`docs/adr/`](adr/) with its alternatives and its reversal cost; a
> decision that turns out wrong gets a new ADR, not a quiet edit.

realm is a keyboard-first, gapless-tiling Wayland desktop environment. It is
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

The projection lives in `realm-core::layout` and is already implemented and
tested against exact-tiling invariants at five resolutions and twelve window
counts.

---

## 2. Component map

```
                          ┌────────────────────────────────────────┐
   L0  contracts          │              realm-core                 │
                          │  ledger · layout · palette · color     │
                          │  keys · state · ipc · glyphs           │
                          └───────────────┬────────────────────────┘
                                          │ (every crate depends on this)
   ┌──────────────────────────────────────┼──────────────────────────────────┐
   │                                      │                                  │
   ▼                                      ▼                                  ▼
┌────────────────┐              ┌──────────────────┐              ┌────────────────┐
│  realm-session  │◀──unix sock──│    realm-ctl      │              │   realm-theme   │
│   (daemon)     │   NDJSON     │      (CLI)       │─────uses────▶│     (lib)      │
│                │              └──────────────────┘              │  palette.toml  │
│  owns RealmState│                                                │   → templates  │
│  WmBackend ────┼──▶ RiverBackend  (phase 1: realm IS the WM)     └───────┬────────┘
│                │    NativeBackend (phase 3)                             │
└───────┬────────┘                                                        │ publishes
        │ broadcasts RealmState                                            ▼
        │                                              gtk.css · kvantum · ANSI ·
        ├──────────────┬──────────────┐                yazi · btop · starship · fuzzel
        ▼              ▼              ▼
┌──────────────┐ ┌───────────┐ ┌────────────┐
│   realm-bar   │ │realm-hecate│ │  realm-odin │        L2  clients
│ layer-shell  │ │ launcher  │ │  ratatui   │
└──────────────┘ └───────────┘ └────────────┘

   L4  integration:  yazi (charon) · btop (horus) · zsh+starship (thoth)
                     xdg-desktop-portal · systemd user units · session entry
```

### Crate responsibilities

| Crate | Kind | Owns | Lands in |
|---|---|---|---|
| `realm-core` | lib | Ledger, layout projection, palette, keymap, portable IPC wire values/codec/version, glyph inventory; no path or socket operations | **M0 — done** |
| `realm-theme` | lib | `palette.toml` → normalized outputs in one sealed immutable generation. Serial rendering, validation, future-launch pointer publication, and read-only generation diff; no pointer-switch reload | M1 |
| `realm-control` | Linux-only lib | Shared retained endpoint capabilities; later #41 client/server socket transport. Fails non-Linux compilation explicitly | M2 |
| `realm-ctl` | bin (`realmctl`) | `realmctl theme/orbit/ledger/doctor/run`. The scriptable surface; uses `realm-control`, never `realm-session` | M1–M2 |
| `realm-session` | bin (`realm-wm`) | Holds `RealmState`, drives a `WmBackend`, serves the control socket through `realm-control`, launches clients. Under river it *is* the window manager, hence the binary name | M2 |
| `realm-bar` | bin | Layer-shell bar, which-key strip, mode badge, chord echo | M2 |
| `realm-hecate` | bin | Layer-shell fuzzy launcher (`nucleo`) | M4 |
| `realm-odin` | bin | `ratatui` agent-harness TUI | M4 |
| `realm-compositor` | bin | Smithay compositor; `NativeBackend` for `realm-session` | M5 |

Crates join the Cargo workspace when they gain a real implementation, so a
fresh clone always builds.

---

## 3. Decisions

Each row links to its ADR. "Reversal" is the honest cost of changing our mind.

| # | Decision | Rationale in one line | Reversal |
|---|---|---|---|
| [0001](adr/0001-ledger-as-single-source-of-truth.md) | Ledger + pure projection | Undo, focus and damage-tracking all become trivial | Structural — the whole DE assumes it |
| ~~[0002](adr/0002-borrow-a-compositor-first.md)~~ | ~~Ship on **niri** first~~ | *Superseded by 0013.* Kept for the evidence that moved us | — |
| [0013](adr/0013-river-window-management-backend.md) | Be the window manager for **river 0.4** | river 0.4 removed window management from the compositor; `river-window-management-v1` gives exact position, dimensions, node ordering, hide/show and borders — realm's ledger model exactly | Low — hidden behind `WmBackend` |
| [0003](adr/0003-session-daemon-owns-state.md) | A session daemon owns state; clients subscribe | Bar/launcher stay dumb; swapping the compositor changes one file | Low |
| [0004](adr/0004-ndjson-control-socket.md) | Newline-delimited JSON over a unix socket | Scriptable with `socat`; partial frames can't be misread | Low |
| [0005](adr/0005-palette-toml-single-source.md) | One `palette.toml` → generated themes | No colour is written down twice; contrast is derived, not filtered. Its mutable activation clauses are superseded by 0017 | Low |
| [0017](adr/0017-immutable-theme-activation-generations.md) | Publish sealed immutable theme generations | Launches pin a validated digest-bound tree; pointer commits affect future launches only and never reload | Medium |
| [0006](adr/0006-oklab-contrast-not-filters.md) | Perceptual contrast derivation, gamut-capped | A `contrast()` filter costs a fullscreen pass and rotates hues | Low |
| [0007](adr/0007-reuse-yazi-btop-starship.md) | Reuse yazi / btop / zsh+starship rather than rewrite | charon and horus are ~90% theme + keymap. Rewrites cost years and lose features | Low |
| [0008](adr/0008-layer-shell-rendering-stack.md) | `smithay-client-toolkit` + `tiny-skia` + `cosmic-text` | Pure Rust, no GPU context for a 32px bar, real font fallback | Medium |
| [0009](adr/0009-no-animation-budget.md) | Zero animations; a frame budget, enforced in CI | "Snappy" is a number, not an adjective | Low |
| [0010](adr/0010-nix-flake-as-reference-build.md) | Nix flake is the reference build; native deb/rpm packaging is tracked | Root flake plus planned overlap-consistency checks; no shared package-metadata generator | Medium |
| [0011](adr/0011-session-integration-contract.md) | Session startup owns the D-Bus/systemd environment handshake | The #1 way minimal Wayland desktops break for users | Low |
| [0012](adr/0012-font-fallback-is-a-contract.md) | Glyph inventory + startup probe + ASCII fallbacks | A glyph-heavy DE must never ship tofu | Low |

### The compositor question, stated plainly

The brief asks for a Smithay compositor eventually. We agree — and we are not
starting there. Building one first means a year before anyone can log in, and
the interesting design work, the ledger, is testable without it.

So `realm-session` talks to a `WmBackend` trait. In phase 1, `RiverBackend`
implements it by **being river's window manager**: river 0.4 removed all window
management from the compositor and defers it to an external process over
`river-window-management-v1`, which offers exact positions and dimensions,
explicit node ordering, hide/show, focus control and compositor-drawn borders.
That is realm's model rather than an approximation of it — the ledger drives real
pixels from M2. In phase 3, `NativeBackend` implements the same trait in-process
against `realm-compositor`, and nothing above the trait changes.

This replaces an earlier decision to ship on niri, whose scrollable-tiling model
could only approximate the triptych and had no equivalent for stow. ADR 0002
records that reasoning and the mapping table that argued us out of it; ADR 0013
records where we landed.

Two things this costs us, stated plainly:

- **realm must *implement* five companion protocols, not merely call them.**
  `river-layer-shell-v1` (without which the bar never appears at all, since
  layer-shell under river is the window manager's job), `river-xkb-bindings-v1`
  (the whole keymap), `river-input-management-v1`, `river-xkb-config-v1`
  (keyboard layouts, otherwise frozen at whatever river started with) and
  `river-libinput-config-v1` (without which a laptop has no tap-to-click and no
  way to get one — under river 0.4 the window manager *is* the input
  configuration). M2 is scoped accordingly.
- **`realm-session` joins the input path with a liveness requirement.** The
  protocol has an `unresponsive` error and finite input buffering, so a stall is
  a session failure rather than a slow frame. That promotes §4's budgets from
  performance goals to correctness requirements.

On stability: the protocol is **declared stable** as of river 0.4.0 with a
forward-compatibility pledge to 1.0.0. The residual risk is trust in a single
maintainer of a pre-1.0 project — real, but smaller than an unstable
classification would imply. We pin a tested river and treat a protocol bump as a
tracked event.

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
| Key press → new geometry submitted | < 4 ms | Projection is pure integer maths; benchmarked in CI. **Under river this is a correctness bound, not a comfort one**: realm is on the input path and a stalled window manager is a dead session |
| State change → bar redraw | < 8 ms | Damage-tracked; `RealmState::renders_same_as` drops no-op frames |
| Bar idle CPU | ~0% | The bar owns no timer and redraws only on a state change |
| Sampler wakeups | 1 Hz, one thread, in `realm-session` | cpu, mem, gpu and net throughput are rates over counters with no kernel event behind them. One shared sampler off the input path is the single documented exception to the no-timers rule; the clock ticks to the next minute boundary, not every second |
| Cold session start → usable | < 900 ms | No GPU context for the bar, no icon-cache scan, no thumbnailer |
| `realmctl theme apply` | < 150 ms | Templates rendered serially, then one complete generation is validated, sealed, fsynced, and selected for future launches; no mutable-target shortcut or reload |

**Robust — the failure modes we refuse to ship** (see
[docs/PITFALLS.md](PITFALLS.md) for the full register):

- Hairline cracks between tiles from rounding — *closed by exact-tiling tests.*
- Tofu glyphs on a machine without Nerd Fonts — *closed by the glyph probe.*
- Portals that hang because `WAYLAND_DISPLAY` never reached the D-Bus
  activation environment — *closed by the session contract (ADR 0011).*
- A partial or mismatched theme becoming selectable — *closed by sealed
  manifest validation and an atomic `current` commit.* Existing processes remain
  pinned; the pointer switch changes future launches only.
- A crashed bar taking the session with it — *clients are restartable units; the
  session daemon outlives them.*
- An unreadable palette after a contrast tweak — *closed by `palette lint` in CI.*

---

### The rest of the desktop

A DE is judged on the two dozen services it does *not* write: notifications,
portals, lock, polkit, audio, networking, secrets. That surface is inventoried,
researched and prioritised in
[docs/INTEGRATION-SURFACE.md](INTEGRATION-SURFACE.md), with the evidence in
[docs/integration/](integration/). Two findings from it changed this document:
the bar cannot be timer-free (§4), and river has five protocols realm must serve
rather than three (§3).

## 5. Target platforms

Target plan and current evidence:

| Platform | Delivery | Notes |
|---|---|---|
| **NixOS / Nix** | Root flake imports packages, `nixosModules.realm`, `homeManagerModules.realm` | Reference build with committed `flake.lock`; lock updates are reviewed dependency changes. A NixOS VM test boots the session and asserts the bar appears |
| **Ubuntu** 24.04 LTS + | tracked native `packaging/debian/` files | The MSRV is set by the locked dependency graph's edition floor (currently Rust 1.85), not by glibc. If Realm distributes prebuilt binaries, glibc will constrain their compatibility separately. |
| **Compositor** | River `>= 0.4.0`, with a source selected per target | Fedora 44 has an official native candidate; availability is not compatibility evidence. Ubuntu and Nix remain governed by ADR 0010/0013 |
| **Fedora 44 (pre-alpha)** | exactly one pinned Cargo-smoke lane plus exactly one retained-source RPM-build lane | The RPM is built but not clean-installed; graphical login and SELinux behavior remain unverified |

Anything else is best-effort. The root flake is the Nix reference definition;
the distro packages are tracked native definitions whose common contract must
be checked mechanically before release.

---

### Binary names

Crate names and installed binary names deliberately differ where the executable
role needs to be immediately clear to a user:

| Crate | Binary | Why |
|---|---|---|
| `realm-session` | `realm-wm` | `realm-session` is already the session *wrapper* script the display manager runs. Two different things called `realm-session` in one `ps` output is a support burden, and under river the daemon genuinely is the window manager |
| `realm-ctl` | `realmctl` | SPEC 0006 and SPEC 0025 define one unambiguous command spelling across every distribution and document |

The CLI is always `realmctl`; no forwarding alias is part of the product.

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

realm is built **spec-first** (standing order S14): behaviour is written down in
[`docs/specs/`](specs/) — and any decision in [`docs/adr/`](adr/) — before it is
implemented, then covered by happy-path tests that are watched to fail first.
Layout and colour maths are the standing exception and get invariant tests
instead, because example tests pass while those algorithms are subtly wrong.

Work is tracked in GitHub milestones M0–M6; see [docs/MVP.md](MVP.md) for the
cut line and [docs/ROADMAP.md](ROADMAP.md) for what each milestone contains.
Issues that need a human judgement call — hardware access, a design trade-off,
a licence question — carry the **`needs-human`** label and are listed in the
README so they are never buried.
