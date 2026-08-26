# Roadmap — M0 to M6

The cut line is in [MVP.md](MVP.md) and outranks this page: if the two ever
disagree, MVP.md is right and this file is stale. What follows is the same
milestone table with the reasoning restored — what each milestone is *for*, what
work sits inside it, the single thing that decides whether it is finished, and
what it releases downstream.

**[M3](#m3--daily-drivable) is the MVP.** M0 through M3 are the critical path;
nothing in M4 and beyond blocks it. Work in later milestones is real, planned
work — it is simply not allowed to jump the queue.

---

| | Milestone | Goal in one sentence | Exit criterion | Status |
|---|---|---|---|---|
| [M0](#m0--foundations) | Foundations | Agree the shape and write the contracts down. | `cargo test` green in CI; architecture reviewed | **in progress** |
| [M1](#m1--theming-pipeline) | Theming pipeline | One palette file retints the whole desktop. | One palette edit visibly retints GTK, terminal, yazi and btop | planned |
| [M2](#m2--session-and-bar) | Session and bar | The ledger drives real pixels, and the bar shows it. | Bar reflects live orbit/focus/mode changes, and the reference triptych geometry is pixel-exact on river | planned |
| [M3](#m3--daily-drivable) | **Daily-drivable — the MVP** | Someone can use helm as their only desktop for a week. | A fresh NixOS/Ubuntu/Fedora box logs into helm and passes `doctor` | planned |
| [M4](#m4--native-clients) | Native clients | Retire the stopgaps. | fuzzel, the toolkit file dialog and the terminal-run agent runner are gone | planned |
| [M5](#m5--the-helm-compositor) | helm compositor | The ledger runs the screen directly. | `NativeBackend` passes the same session tests as `NiriBackend` | planned |
| [M6](#m6--polish) | Polish | Hold the budgets on modest hardware; finish the edges. | Frame budgets held on a 2015-era laptop | planned |

---

## M0 — Foundations

**Goal.** Decide the architecture, write the contracts every other crate will
depend on, and make the repository something a stranger can understand.

**Workstreams**

- `helm-core`: the ledger and its undo history; the layout projection and its
  exact-integer partition; OKLab colour derivation and `Palette::lint`; the
  keymap and mode model; the glyph inventory and its ASCII fallbacks; the NDJSON
  IPC types and `PROTOCOL_VERSION`.
- The written record: [ARCHITECTURE](ARCHITECTURE.md), [MVP](MVP.md),
  [PITFALLS](PITFALLS.md), the decision register in [`adr/`](adr/), the seam
  contracts in [INTERFACES.md](INTERFACES.md), and the spec process in
  [`specs/`](specs/).
- CI: `fmt`, `clippy -D warnings`, `test`, on every push.
- Repository furniture: README, CONTRIBUTING, licences, issue and PR templates.

**Exit criterion.** `cargo test` is green in CI and the architecture has been
read and argued with rather than merely written.

**Unblocks.** Everything. `helm-core` is the only crate every other crate
depends on, and its types are what stop two components being invented in
parallel with incompatible ideas of the same thing.

---

## M1 — Theming pipeline

**Goal.** Make `palette.toml` the only place a colour is written down, and make
one edit to it reach every themed surface without a relogin.

**Workstreams**

- `helm-theme`: template engine, the template set in `configs/templates/`,
  atomic writes (render to a temporary file, `rename(2)`, then reload), and the
  reload fan-out.
- Templates for the surfaces the MVP needs: `gtk.css` for GTK 3 and 4,
  libadwaita named colours, `qt6ct` colour scheme, the terminal's 16-colour ANSI
  scheme, `yazi`, `btop`, `starship`, `fuzzel`.
- `helm ctl theme apply` and `helm ctl theme lint` — the first two subcommands
  of the CLI, and its argument surface.
- A CI check that no colour literal exists outside `palette.toml`.

**Exit criterion.** Change one accent in `palette.toml`, run
`helm ctl theme apply`, and watch GTK, the terminal, yazi and btop all change —
in under 150 ms, with no half-applied intermediate state.

**Unblocks.** M2's bar (which reads the same palette), and the "one coherent
theme" line of the MVP. Also closes two rows of the failure register: the
half-applied theme, and the colour written down twice.

---

## M2 — Session and bar

**Goal.** Put helm's state under one owner, and put that state on screen.

**Workstreams**

- `helm-session`: owns `HelmState`, serves the control socket at
  `$XDG_RUNTIME_DIR/helm/ctl.sock`, broadcasts state to subscribers, and owns
  the lifecycle of its clients so a crashed bar does not take the session with
  it.
- The `WmBackend` trait — sketched with real signatures in
  [INTERFACES.md](INTERFACES.md) — and `RiverBackend`, its first
  implementation. river 0.4 removed window management from the compositor and
  defers it to an external process over `river-window-management-v1`, which
  offers exact positions and dimensions, explicit node ordering, hide/show,
  focus control and compositor-drawn borders. helm *is* that process. The
  ledger's rectangles become real ones, not an approximation of them.
  ([ADR 0013](adr/0013-river-window-management-backend.md), superseding
  [0002](adr/0002-borrow-a-compositor-first.md))
- `helm-bar`: layer-shell bar at 32 px — orbit runes, layout indicator, mode
  badge, chord echo, focused title, the right-hand modules and the clock — plus
  the which-key strip and the `?` grimoire sheet.
- `helm ctl orbit` and `helm ctl ledger`: the scriptable surface over the
  socket.
- The rendering stack decided in
  [ADR 0008](adr/0008-layer-shell-rendering-stack.md), including real font
  fallback so the glyph probe has something to probe.

**Exit criterion.** Running on river, changing orbit, focus or mode is
reflected in the bar — event-driven, with idle CPU at approximately zero and no
redraw on a timer — and the triptych geometry on screen is pixel-exact against
the reference measurements, not merely close.

**Unblocks.** M3, which is mostly integration work on top of a session that
already runs. Also the first honest measurement against the frame budgets in
[ARCHITECTURE §4](ARCHITECTURE.md#4-what-robust-and-snappy-mean-here).

---

## M3 — Daily-drivable

> **This is the MVP.** The whole test: *one person can log into helm and use it
> as their only desktop for a week without reaching for another DE.*

**Goal.** Turn a session daemon and a bar into something a person can log into
on a machine that is not the author's.

**Workstreams**

- Session entry: the `.desktop` session file, the startup wrapper, systemd user
  units, and the D-Bus and systemd environment handshake from
  [ADR 0011](adr/0011-session-integration-contract.md). This is the single most
  common way minimal Wayland desktops break for users.
- Portals: `xdg-desktop-portal` wiring, a backend dependency that is actually
  installed, `XDG_CURRENT_DESKTOP=helm` exported both ways, file dialogs and
  screen share verified in a browser and an Electron app.
- The reused tools, themed and rekeyed: charon (yazi), horus (btop), thoth (zsh
  and starship), a terminal on the generated ANSI theme, and fuzzel dressed as
  hecate.
- Packaging: the Nix flake as the reference build with `nixosModules.helm` and
  `homeManagerModules.helm`, plus the `.deb` and `.rpm` generated from the same
  metadata. A NixOS VM test that boots the session and asserts the bar appears.
- A **vendored, pinned river 0.4.x** in every package. Ubuntu and Fedora ship
  river 0.3.x or river-classic, neither of which speaks the window-management
  protocol. Vendoring puts Zig in the packaging pipeline only, never in helm's
  own workspace — and it is an
  [open `needs-human` question](../README.md#needs-a-human).
- `helm ctl doctor`: checks the environment handshake, the portal backend, the
  cursor theme, the font stack and the socket, and says what is wrong in plain
  words before the user has to file a bug.
- Idle and lock handling — **blocked on an open
  [`needs-human`](../README.md#needs-a-human) question**: which lock screen
  ships. A laptop that does not lock on lid-close is not daily-drivable.
- Install documentation for all three distributions.

**Exit criterion.** A fresh NixOS, Ubuntu and Fedora box each log into helm and
pass `helm ctl doctor` clean, with no manual steps beyond installing the
package.

**Unblocks.** Everything after this point is improvement rather than arrival.
M4 and M5 replace parts of a working desktop; before M3 there is nothing to
replace parts of.

---

## M4 — Native clients

**Goal.** Retire the stopgaps, each of which was chosen to be swappable.

**Workstreams**

- `helm-hecate`: layer-shell fuzzy launcher on `nucleo`, over `PATH`, desktop
  entries, user "spells" and helm commands. Replaces themed fuzzel.
- `helm-odin`: the `ratatui` agent-harness TUI — familiar table, timestamped
  log, status footer. Nothing off the shelf matches it.
- charon *portal*: the open dialog, via
  `xdg-desktop-portal-termfilechooser` pointed at yazi in a centred floating
  terminal. Replaces the toolkit's own dialog.
- urania: the orrery pane. The one deliberate ornament, and last in the queue
  because it is.

**Exit criterion.** No stopgap remains in the default session, and removing
each one required touching only its seam — a config entry or a trait
implementation — not its callers.

**Unblocks.** Nothing downstream depends on M4; it exists to close the gap
between the shipped desktop and the design.

---

## M5 — The helm compositor

**Goal.** Stop projecting helm's window model onto someone else's, and let the
ledger drive the screen directly.

**Workstreams**

- `helm-compositor` on Smithay: scene graph, input, output management,
  XWayland.
- `NativeBackend`: the second implementation of the same `WmBackend` trait
  `NiriBackend` implements, in-process rather than over IPC.
- Retiring the vendored river and the dependency on an unstable protocol.
  `river-window-management-v1` is a good bargain, not a permanent one; owning
  the compositor ends both the vendoring burden and the protocol-bump watch.
- Screen capture and clipboard, now that helm owns the surfaces.

**Exit criterion.** `NativeBackend` passes the same session-level tests as
`RiverBackend`, and the ledger's projection reaches the screen in-process, with
no protocol between the projection and the pixels.

**Unblocks.** The exact-tiling guarantees hold end to end rather than up to the
protocol boundary, and helm stops shipping someone else's compositor in its own
packages.

---

## M6 — Polish

**Goal.** Make the budgets true on hardware nobody would call fast, and finish
the edges deliberately left rough.

**Workstreams**

- Qt and Kvantum theming beyond the `qt6ct` colour scheme: the Kvantum SVG
  generated from the same palette.
- Multi-monitor beyond "it does not break": per-output workareas, orbit
  placement, hot-plug.
- Accessibility: contrast variants exercised across their full range, keyboard
  reachability for everything, an honest account of what a screen reader can and
  cannot see.
- The optional minimal-motion pass: opt-in, opacity-only, at most 120 ms, on
  overlays only. Windows never move or scale. Off by default.
  ([ADR 0009](adr/0009-no-animation-budget.md))
- Performance work against the budgets on a 2015-era laptop.

**Exit criterion.** Every frame budget in
[ARCHITECTURE §4](ARCHITECTURE.md#4-what-robust-and-snappy-mean-here) is held on
a 2015-era laptop, measured rather than asserted.

**Unblocks.** Nothing. M6 is where "snappy" stops being a claim about the
author's machine.

---

## How the queue is enforced

The sequencing rules in [MVP.md](MVP.md#sequencing-rules) are what keep this
order from being a suggestion:

1. **Contracts before implementations** — `helm-core` types land before the
   crate that consumes them.
2. **Stopgaps must be swappable** — every stopgap sits behind a config or a
   trait, never a hardcoded call. The name `river` may not appear outside
   `crates/helm-session/src/backend/`, `packaging/` and `docs/`. This
   confinement is the only reason M4 and M5 are affordable.
3. **Nothing merges without a test** — layout maths gets unit tests, live
   sockets get integration tests, anything touching a distribution gets a CI job
   on that distribution.
4. **The budget is a gate, not a goal** — missing a frame budget is a
   regression, not a trade-off.

Milestones are GitHub milestones and every unit of work is an issue on one of
them. Issues that need a person's judgement carry the **`needs-human`** label
and are listed on the [front page](../README.md#needs-a-human), never buried.
