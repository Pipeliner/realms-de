<h1 align="center">✦ helm</h1>

<p align="center">
  <strong>a keyboard-first, gapless-tiling, Rust-first Wayland desktop environment</strong><br>
  <em>zero animations, one palette file · space and magic, no wasted pixels</em>
</p>

<p align="center">
  <a href="https://github.com/Pipeliner/realms-de/actions"><img alt="ci" src="https://img.shields.io/github/actions/workflow/status/Pipeliner/realms-de/ci.yml?label=ci&style=flat-square&labelColor=0a0c15&color=7fd4c1"></a>
  <a href="#licence"><img alt="licence: MIT OR Apache-2.0" src="https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-a692ec?style=flat-square&labelColor=0a0c15"></a>
  <a href="Cargo.toml"><img alt="MSRV 1.85" src="https://img.shields.io/badge/msrv-1.85-d9b06a?style=flat-square&labelColor=0a0c15"></a>
  <a href="docs/ROADMAP.md"><img alt="status: pre-alpha, milestone M0" src="https://img.shields.io/badge/status-pre--alpha%20(M0)-a3bff2?style=flat-square&labelColor=0a0c15"></a>
</p>

<p align="center">
  <img src="docs/assets/hero.svg" alt="Concept rendering, not a screenshot. The intended helm desktop: a 32-pixel bar carrying six runic orbits, a layout indicator, a mode badge and a clock; below it a gapless triptych of five panes — odin, thoth, hermes, horus and urania — separated by one-pixel violet seams; a which-key strip along the bottom." width="100%">
</p>

<p align="center">
  <sub><strong>Concept rendering — not a screenshot.</strong> No part of this image was produced by running helm;
  it is hand-drawn SVG to the same measurements the compositor will use.<br>
  It will be replaced by a real screen capture once there is something to capture. See <a href="#status">Status</a>.</sub>
</p>

---

## What makes it different

- **The ledger is the truth.** Window positions are never stored — they are
  projected from an ordered list, on demand. Undo is not a stack of inverse
  operations; it restores an older ledger, and the screen comes back exactly as
  it was. [ADR 0001](docs/adr/0001-ledger-as-single-source-of-truth.md)

- **No colour outside `palette.toml`.** `palette.toml` is the single source of
  truth for the bar, the terminal, GTK, Qt, yazi, btop and the prompt. Contrast
  is *derived* in perceptual colour space, not applied as a fullscreen filter,
  so accents keep their hue instead of rotating. Nothing downstream is permitted
  a colour literal. [ADR 0005](docs/adr/0005-palette-toml-single-source.md)

- **Snappy is a number.** There are no animations, and there is a published
  frame budget for every path that can feel slow: key press to new geometry
  under 4 ms, state change to bar redraw under 8 ms, cold session start under
  900 ms. Budgets are gates in CI, not aspirations.
  [ADR 0009](docs/adr/0009-no-animation-budget.md) ·
  [ARCHITECTURE §4](docs/ARCHITECTURE.md#4-what-robust-and-snappy-mean-here)

- **Proven tools, kept behind seams.** charon is yazi, horus is btop, thoth is
  zsh with starship — themed and rekeyed from the same palette, each sitting
  behind a config or a trait so it can be retired without touching its callers.
  The compositor is the same bargain: helm is the *window manager* for river
  0.4, which hands window management to an external process, so the ledger
  drives real pixels years before `helm-compositor` exists.
  [ADR 0007](docs/adr/0007-reuse-yazi-btop-starship.md) ·
  [ADR 0013](docs/adr/0013-river-window-management-backend.md)

- **Three intended M3 targets, evidence kept honest.** NixOS, Ubuntu and Fedora
  remain the installation targets. Today Fedora 44 is the sole Fedora
  pre-alpha baseline. Its two required pinned-image lanes are a Cargo smoke and
  a retained-source RPM build; the latter builds `Source0` but does not
  clean-install its result. Neither is graphical-session or SELinux acceptance.
  [ADR 0015](docs/adr/0015-fedora-44-pre-alpha-baseline.md)

Behaviour is written down before it is written in Rust: every non-trivial
component gets a spec in [`docs/specs/`](docs/specs/), and its happy-path tests
come from that spec's acceptance criteria before the implementation does.

---

## The one idea

Everything above falls out of a single decision.

<p align="center">
  <img src="docs/assets/ledger.svg" alt="A user action edits the ledger, an ordered list of window ids per orbit with an undo history. A pure projection turns the ledger, a layout and the workarea into a vector of placements: exact integer rectangles that tile the workarea with nothing left over. Those are diffed against the last frame, so only the changed region is submitted." width="100%">
</p>

<p align="center"><sub>A diagram of the model, not of a running system.</sub></p>

A `Ledger` is an ordered `Vec<WinId>` per orbit. Every gesture the user makes —
summon, banish, swap, focus, change layout — edits that list and nothing else.
Geometry is produced by

```rust
fn project(orbit: &Orbit, area: Workarea, params: TriptychParams) -> Vec<Placement>
```

which is pure integer arithmetic: no interior mutability, no clock, no I/O. The
rectangles it returns are partitioned by largest remainder, so they sum to the
workarea *exactly* and there is never a hairline crack between tiles — at 1281×801
as reliably as at 1920×1080.

Because the projection is pure, three properties are free rather than
engineered, and each is held by a test rather than by care:

| Property | Test that would fail if it regressed |
|---|---|
| Undo restores an exact screen | `ledger::tests::undo_restores_the_exact_previous_ledger` |
| Focus never moves a rectangle | `layout::tests::projection_is_pure_and_focus_only_moves_the_flag` |
| An unchanged frame is never drawn | `state::tests::revision_alone_does_not_force_a_redraw` |

The long form, with the component map and the decision register, is in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

---

## Status

**Pre-alpha. Milestone M0 is in progress. There is no desktop environment here yet.**

What exists, honestly:

| | |
|---|---|
| `helm-core` | **Implemented and tested.** Ledger, layout projection, OKLab palette derivation and lint, keymap, glyph inventory and IPC types. [`cargo test`](#try-it) is the current executable evidence. |
| Architecture, MVP cut line, failure register | **Written.** [ARCHITECTURE](docs/ARCHITECTURE.md) · [MVP](docs/MVP.md) · [PITFALLS](docs/PITFALLS.md) |
| Specs and ADRs | **In progress.** [`docs/specs/`](docs/specs/) · [`docs/adr/`](docs/adr/) |
| `helm-theme` | **Implemented and tested pre-alpha library.** It renders and validates sealed generations, but has no user-facing `helmctl theme` command yet. |
| `helm-ctl`, `helm-session`, `helm-bar`, `helm-hecate`, `helm-odin`, `helm-compositor` | **Planned.** The binaries that make a usable desktop do not exist yet. |
| Package/session/portal assets | **Tracked pre-alpha contract.** The repository contains a session entry and wrapper, systemd units, portal configuration, Nix module and native package definitions; none creates a usable Helm desktop yet. |
| Every image in this repository | **Concept art.** Hand-drawn SVG and the design handoff's HTML prototypes. There are no screenshots of helm, because helm does not run yet. |

Crates join the Cargo workspace only when they gain a real implementation, so a
fresh clone always builds. If a crate is not in
[`Cargo.toml`](Cargo.toml), it does not exist yet.

The milestone table — what each of M0 through M6 ships, and what it unblocks —
is in [docs/ROADMAP.md](docs/ROADMAP.md). **M3 is the MVP.**

## Try it

You cannot log into helm yet. A tracked session entry intentionally aborts
because `helm-wm` is not implemented; the bar is also absent, so the
package/session assets are not a usable desktop. `cargo test` exercises the
implemented pre-alpha libraries, not a running desktop.

```console
$ git clone https://github.com/Pipeliner/realms-de
$ cd realms-de
$ cargo test
```

The first release anyone can log into is **M3**, whose cut line is written down
in [docs/MVP.md](docs/MVP.md): log in, tile windows across six orbits, see and
discover the keys, launch things, files, monitor, shell, one coherent theme
across GTK/Qt/TUIs, working portals, packages for NixOS, Ubuntu and Fedora, and
a `helm ctl doctor` that says what is wrong before you file a bug.

---

## Needs a human

Standing order S3: judgement the repository cannot supply is surfaced here, on
the front page, rather than buried in an issue. This is a snapshot of the open
[**`needs-human`**](https://github.com/Pipeliner/realms-de/labels/needs-human)
label at **2026-08-30T06:18:36Z**; follow the label for the live state.

| Issue | What it blocks |
|---|---|
| [#168 — Reconcile generation GC with transferred lifecycle leases and M1/M2 launch sequencing](https://github.com/Pipeliner/realms-de/issues/168) | An accepted lifecycle-lease/GC contract and truthful M1/M2 launch sequencing. |
| [#166 — Specify JSON schemas for helmctl theme lint and diff](https://github.com/Pipeliner/realms-de/issues/166) | The public `theme lint` and `theme diff` JSON contracts. |
| [#135 — Complete exact M1 activation assets and supported-consumer probes](https://github.com/Pipeliner/realms-de/issues/135) | Exact M1 consumer assets and supported-consumer black-box probes. |
| [#134 — Decide supported M1 package sources and catalog migration for Yazi and Starship](https://github.com/Pipeliner/realms-de/issues/134) | The supported Yazi/Starship package-source and catalog-migration policy. |
| [#133 — Specify truthful desktop-entry and D-Bus activation for themed Qt launches](https://github.com/Pipeliner/realms-de/issues/133) | A truthful desktop-entry and D-Bus activation contract for themed Qt launches. |
| [#132 — Reconcile activation launch lifecycle with session teardown and restart](https://github.com/Pipeliner/realms-de/issues/132) | The activation lifecycle, teardown and restart contract. |
| [#35 — Does the 𓂃 prompt sigil survive on the target distros' default fonts, or does `~` become the default?](https://github.com/Pipeliner/realms-de/issues/35) | A verified default for the Egyptian prompt sigil on target installations. |
| [#30 — Template: starship prompt for thoth, with the 𓂃 sigil and its ASCII fallback](https://github.com/Pipeliner/realms-de/issues/30) | The Starship prompt’s default-glyph decision. |
| [#25 — Template: GTK 3, GTK 4 and libadwaita stylesheets](https://github.com/Pipeliner/realms-de/issues/25) | An accepted GTK configuration-location contract. |
| [#24 — Extend the "no colour outside palette.toml" CI guard to templates and generated outputs](https://github.com/Pipeliner/realms-de/issues/24) | A palette-literal guard for templates and generated outputs. |
| [#23 — Add `helmctl theme apply`, `lint` and `diff`](https://github.com/Pipeliner/realms-de/issues/23) | The complete `helmctl theme` interface. |
| [#17 — Configure branch protection on the default branch with the CI checks as required](https://github.com/Pipeliner/realms-de/issues/17) | Enforced required CI checks and branch protection. |
| [#16 — Enable Dependabot alerts and version updates, and create the labels its config references](https://github.com/Pipeliner/realms-de/issues/16) | Dependabot activation and the labels it requires. |

---

## Repo map

```
realms-de/
├─ crates/
│  └─ helm-core/        the contracts: ledger, layout, palette, keys, ipc, glyphs
├─ configs/             shipped configs for the tools helm reuses
│  ├─ templates/          rendered theme inputs for GTK, Qt, TUI and prompt targets
│  └─ portal/             xdg-desktop-portal wiring
├─ packaging/
│  ├─ nix/              Nix package/module/check implementation
│  ├─ debian/           control, rules
│  └─ fedora/           spec
├─ flake.nix            root Nix reference-build entry point
├─ docs/
│  ├─ ARCHITECTURE.md   the shape we are building towards, and why
│  ├─ MVP.md            the cut line — what M3 must do to count
│  ├─ ROADMAP.md        M0–M6: goals, contents, exit criteria
│  ├─ INTERFACES.md     the seams, with real signatures, before the crates exist
│  ├─ PITFALLS.md       the failure register, with the guard for each
│  ├─ specs/            what each component must do, before it does it
│  ├─ adr/              decisions, with alternatives and reversal costs
│  └─ assets/           the diagrams on this page
├─ design/              the design handoff and prototypes, for provenance
├─ .claude/             operational memory, skills, the agentic loop
└─ palette.toml         the one place a colour is written down
```

## Docs

| | |
|---|---|
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | The one idea, the component map, the decision register, the frame budgets, the target platforms. Start here. |
| [MVP.md](docs/MVP.md) | What is in, what is deliberately out, and the test the MVP has to pass. |
| [ROADMAP.md](docs/ROADMAP.md) | M0–M6: one-line goal, workstreams, exit criterion and what each milestone unblocks. |
| [INTERFACES.md](docs/INTERFACES.md) | The seams — `WmBackend`, the template contract, the bar render contract — written with real signatures before the crates that implement them. |
| [PITFALLS.md](docs/PITFALLS.md) | The ways desktop environments break, what helm does about each, and which test would catch a regression. |
| [`docs/specs/`](docs/specs/) | What a component must do, written before it does it. Acceptance criteria become the happy-path tests. |
| [`docs/adr/`](docs/adr/) | Why we chose this over that, and what changing our mind would cost. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to build it, the house style, the spec-first workflow, commit conventions. |
| [SECURITY.md](SECURITY.md) | What is security-relevant here, and how to report something privately. |
| [design/HANDOFF.md](design/HANDOFF.md) | The original design brief. The `.dc.html` prototypes are references, not production code. |

---

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) first — the short version is that
behaviour gets a spec before it gets code, happy-path tests are written and
watched to fail before the implementation, and

```console
$ cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test
```

runs clean before every commit. Anything architectural gets an ADR. Anything
needing a human's judgement gets the `needs-human` label and a statement of the
options, not a quiet guess.

Everyone taking part is held to the [Code of Conduct](CODE_OF_CONDUCT.md).

## How this repository was built

helm is designed and built by AI agents (Claude), under human direction and
review. That is worth stating plainly rather than leaving to be inferred:

- **The design is AI-authored too.** `design/HANDOFF.md` and the `.dc.html`
  prototypes came out of Claude Design, not a human designer. The colours, the
  copy, the glyph choices and the whole aesthetic are an agent's, directed by a
  human. There is no human-authored artefact underneath this that the code is
  merely transcribing.
- **The code, specs, ADRs and this README were drafted by an agent**, then
  reviewed. Architectural decisions with real alternatives were put to a human
  and decided by one — the choice to be river's window manager rather than ship
  on niri is the clearest example, and ADR 0002 is kept superseded-but-intact so
  that reasoning can be audited.
- **Agents check each other.** Several claims in this repository were wrong when
  first written and were caught by a second agent reading the primary source
  rather than the first agent's summary: a declared MSRV that could not build, a
  river protocol mapping that overstated what the protocol offered, and a
  stability claim taken from a tracking issue that predated the release. The
  corrections are in the git history rather than smoothed over.
- **Claims here are meant to be checkable.** Where this README says something is
  tested, it names the test. Where something does not exist, it says so. If you
  find a claim that cannot be checked against the repository, that is a bug —
  please file it.
- **Nothing here has run on real hardware.** No maintainer has logged into helm,
  because there is not yet a session to log into.

## Credits

helm reuses good work rather than repeating it, and owes a debt to
[river](https://codeberg.org/river/river) — whose 0.4 window-management protocol
is what lets the ledger drive real pixels this early —
[Smithay](https://smithay.github.io/),
[yazi](https://yazi-rs.github.io/), [btop](https://github.com/aristocratos/btop),
[starship](https://starship.rs/), [fuzzel](https://codeberg.org/dnkl/fuzzel),
[foot](https://codeberg.org/dnkl/foot), [nucleo](https://github.com/helix-editor/nucleo)
and [ratatui](https://ratatui.rs/). Type is IBM Plex Mono. The runes are Elder
Futhark, U+16A0.

## Licence

Dual-licensed, at your option, under either

- Apache License, Version 2.0 — [LICENSE-APACHE](LICENSE-APACHE)
- MIT licence — [LICENSE-MIT](LICENSE-MIT)

Unless you state otherwise, any contribution you intentionally submit for
inclusion in this work, as defined in the Apache-2.0 licence, shall be
dual-licensed as above, with no additional terms or conditions.

<p align="center"><sub>© 2026 the helm contributors · ᚠ ᚢ ᚦ ᚨ ᚱ ᚲ</sub></p>
