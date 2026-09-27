# The MVP cut line

Accepted 2026-09-27 scope correction: [ADR 0023](adr/0023-reuse-first-session-theme-mvp.md)
and [SPEC 0030](specs/0030-reuse-first-session-theme-mvp.md). Select themes once
at login; apply prepares the next login. Compare Waybar before more custom bar
internals. This is a target contract, not a claim the migration is implemented.

**MVP = one person can log into realm and use it as their only desktop for a
week without reaching for another DE.**

That is the whole test. Everything below is judged against it, and anything that
does not serve it waits — however good it would look in a screenshot.

---

## In

| # | Capability | Why it is in | Crate / component |
|---|---|---|---|
| 1 | Log in and get a session | Without this there is no desktop | session entry, systemd units |
| 2 | Tile windows: summon, banish, focus, swap, six orbits, triptych + mono | This *is* the window manager — literally, under river 0.4 | `realm-core` + `realm-session` |
| 3 | See state: bar with orbits, layout, mode, title, clock, cpu/mem/net/battery | A tiling WM without a bar is unusable for a week | Current bar; Waybar comparison under SPEC 0030 |
| 4 | Discover keys: which-key strip + `?` grimoire | Chords no one can remember are chords no one uses | `realm-bar` |
| 5 | Launch things | Terminal, browser, anything on `PATH` | fuzzel themed as hecate |
| 6 | A terminal that looks like realm | Where the week is actually spent | foot + generated ANSI theme |
| 7 | Files: charon | yazi + realm keymap and theme | `configs/yazi` |
| 8 | Monitor: horus | btop + generated theme | `configs/btop` |
| 9 | Shell: thoth | zsh + starship, `nav@caldera :: ~%` by default | `configs/zsh` |
| 10 | One coherent session theme across GTK, Qt and TUIs; changes take effect next login | The difference between a DE and a pile of programs | Existing templates + supported upstream configuration |
| 11 | Portals work: FileChooser, ScreenCast, **Settings**, and **Inhibit routed to `none`** | Browsers and Electron apps are non-negotiable in a work week. Settings is what stops every GTK4 and Flatpak app rendering light; Inhibit must be `none` or the gtk backend claims success and the screen blanks mid-call | `configs/portal` + ADR 0011 |
| 12 | Install on NixOS, Ubuntu and Fedora with River 0.4-compatible, target-specific packaging; Fedora uses its official native candidate | The stated targets | `packaging/` |
| 13 | `realmctl doctor` | Tells the user what is wrong before they file a bug | `realm-ctl` |

## Out — deliberately, for now

| Deferred | Stopgap in MVP | Lands in |
|---|---|---|
| `realm-compositor` (Smithay) | river 0.4 via `RiverBackend`, with realm as its window manager | Shelved; only reconsider for an unmet product need |
| `realm-hecate` native launcher | themed fuzzel | M4 |
| `realm-odin` agent harness | run the agent runner in a terminal | M4 |
| urania orrery pane | — (the one pure-ornament pane) | M4 |
| charon *portal* open dialog | the toolkit's own dialog, themed | M4 |
| Kvantum / Qt theming beyond `qt6ct` colours | qt6ct colour scheme only | M6 |
| Minimal-motion pass | none — v1 is motionless by design | M6 |
| Multi-monitor beyond "it doesn't break" | single output is the tested path | M6 |
| Runtime keyboard-layout switching | preserve the keymap/layout River created for each keyboard | Post-MVP input contract |
| In-incarnation recovery after an admitted policy response reports `Io` or `Unsupported` | fail closed, let supervision restart `realm-session`, and relearn compositor authority through River replay; emit no ordinary completion/effect from the failed transaction | Post-MVP transaction-recovery contract |

---

## Milestones

M4/M5 below are shelved options, not commitments to replace working upstream
programs. Per-launch theme provenance/leases, fresh-Exec-only activation,
generation-aware live upgrade and a custom damage-tracking benchmark harness
are not MVP gates. Preserve functioning code until replacements are verified.

| | Milestone | Ships | Done when |
|---|---|---|---|
| **M0** | Foundations | `realm-core`, CI, docs, ADRs, repo furniture | `cargo test` green in CI; architecture reviewed |
| **M1** | Session theming | Reused templates/configuration, `realmctl theme apply/lint` | Coherent GTK/Qt/terminal/Yazi/btop/shell theme selected at login; apply affects next login only, with actual consumer evidence |
| **M2** | Session and bar | `realm-session` + `RiverBackend`, and the four companion protocols Realm must use under River (`river-layer-shell-v1`, `river-xkb-bindings-v1`, `river-input-management-v1`, `river-libinput-config-v1` — the latter two apply fixed keyboard repeat and support-gated tap-to-click while preserving all other device preferences), plus `realm-bar` | Bar reflects live orbit/focus/mode changes, and the reference triptych geometry is pixel-exact on River |
| **M3** | **Daily-drivable** | Session entry, portals, packaging, install docs | A fresh NixOS/Ubuntu/Fedora box logs into realm and passes `doctor` |
| **M4** | Shelved native-client options | Only justified by an unmet need | No automatic retirement of fuzzel or toolkit dialogs |
| **M5** | Shelved compositor option | River remains the dependency | No mandatory custom compositor |
| **M6** | Polish | Qt/Kvantum, multi-monitor, a11y, optional minimal motion | Frame budgets held on a 2015-era laptop |

**M3 is the MVP.** M0–M3 is the critical path; nothing in M4+ blocks it.

README launch evidence includes real screenshots captured from a running Realm
session. Capture the tiled desktop and key-discovery UI with ordinary sample
applications, inspect each image for working layout, readable text, and visible
state, and record the source commit and capture environment alongside the assets.
Replace the README concept hero only after that inspection; captions must state
what was actually running. A VM or headless compositor capture is acceptable
when identified as such. Use a clean demo session without personal data.
Capture dimensions must come from the PNG IHDR, not the requested VM mode.
Preserve original capture evidence; document any discovered metadata error
alongside it instead of silently rewriting historical provenance.

For the launch slice, bounded fail-closed restart is the recovery mechanism for
an uncertain post-admission compositor result. Neutral repair turns,
rollback-to-continue, continuing diagnostics, and exhaustive shutdown-discard
states are explicitly not M2/M3 gates.

Security checks and security-hardening review are deferred until after the MVP.
They are tracked as post-MVP work and do not gate the M0–M3 critical path.

---

## Sequencing rules

[SPEC 0031](specs/0031-mvp-launch-delivery.md) governs the approved three delivery
lanes, draft/full CI split, dependency freeze, bounded reuse experiments,
actionable doctor scope and shared acceptance journey. All three distros remain.

1. **Contracts before implementations.** `realm-core` types land before the crate
   that consumes them, so two components are never invented in parallel.
2. **Stopgaps must be swappable.** Every stopgap (fuzzel, river, btop) sits
   behind a config or a trait, never a hardcoded call. The name `river` may not
   appear outside `crates/realm-session/src/backend/`, `packaging/` and `docs/`.
3. **Nothing merges without a test.** Layout maths gets unit tests; anything
   touching a live socket gets an integration test; anything touching a
   distro gets a CI job on that distro.
4. **Measure before selecting.** SPEC 0030's Waybar comparison reports the
   [ARCHITECTURE.md §4](ARCHITECTURE.md) budgets and any misses. Do not reject
   GTK on predicted costs or silently waive a measured regression; custom
   renderer internals and benchmark-harness development are not MVP gates.
5. **All packaging builds and verification run in CI only.** This includes Nix
   and native packages, archive/vendor/provenance generation or rebinding, and
   install verification. Local development does not install packaging
   toolchains or treat their absence as a blocker; the matching CI job supplies
   authoritative evidence.
