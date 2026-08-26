# The MVP cut line

**MVP = one person can log into helm and use it as their only desktop for a
week without reaching for another DE.**

That is the whole test. Everything below is judged against it, and anything that
does not serve it waits — however good it would look in a screenshot.

---

## In

| # | Capability | Why it is in | Crate / component |
|---|---|---|---|
| 1 | Log in and get a session | Without this there is no desktop | session entry, systemd units |
| 2 | Tile windows: summon, banish, focus, swap, six orbits, triptych + mono | This *is* the window manager — literally, under river 0.4 | `helm-core` + `helm-session` |
| 3 | See state: bar with orbits, layout, mode, title, clock, cpu/mem/net/battery | A tiling WM without a bar is unusable for a week | `helm-bar` |
| 4 | Discover keys: which-key strip + `?` grimoire | Chords no one can remember are chords no one uses | `helm-bar` |
| 5 | Launch things | Terminal, browser, anything on `PATH` | fuzzel themed as hecate |
| 6 | A terminal that looks like helm | Where the week is actually spent | foot + generated ANSI theme |
| 7 | Files: charon | yazi + helm keymap and theme | `configs/yazi` |
| 8 | Monitor: horus | btop + generated theme | `configs/btop` |
| 9 | Shell: thoth | zsh + starship, `nav@caldera :: 𓂃%` | `configs/zsh` |
| 10 | One coherent theme across GTK, Qt and TUIs | The difference between a DE and a pile of programs | `helm-theme` + `helm-ctl theme apply` |
| 11 | Portals work: file dialogs, screen share | Browsers and Electron apps are non-negotiable in a work week | `configs/portal` + ADR 0011 |
| 12 | Install on NixOS, Ubuntu and Fedora, with a vendored pinned river | The stated targets | `packaging/` |
| 13 | `helm ctl doctor` | Tells the user what is wrong before they file a bug | `helm-ctl` |

## Out — deliberately, for now

| Deferred | Stopgap in MVP | Lands in |
|---|---|---|
| `helm-compositor` (Smithay) | river 0.4 via `RiverBackend`, with helm as its window manager | M5 |
| `helm-hecate` native launcher | themed fuzzel | M4 |
| `helm-odin` agent harness | run the agent runner in a terminal | M4 |
| urania orrery pane | — (the one pure-ornament pane) | M4 |
| charon *portal* open dialog | the toolkit's own dialog, themed | M4 |
| Kvantum / Qt theming beyond `qt6ct` colours | qt6ct colour scheme only | M6 |
| Minimal-motion pass | none — v1 is motionless by design | M6 |
| Multi-monitor beyond "it doesn't break" | single output is the tested path | M6 |

---

## Milestones

| | Milestone | Ships | Done when |
|---|---|---|---|
| **M0** | Foundations | `helm-core`, CI, docs, ADRs, repo furniture | `cargo test` green in CI; architecture reviewed |
| **M1** | Theming pipeline | `helm-theme`, templates, `helm ctl theme apply/lint` | One palette edit visibly retints GTK, terminal, yazi and btop |
| **M2** | Session and bar | `helm-session` + `RiverBackend`, and the three protocols helm must *serve* under river (`river-layer-shell-v1`, `river-xkb-bindings-v1`, `river-input-management-v1`), plus `helm-bar` | Bar reflects live orbit/focus/mode changes, and the reference triptych geometry is pixel-exact on river |
| **M3** | **Daily-drivable** | Session entry, portals, packaging, install docs | A fresh NixOS/Ubuntu/Fedora box logs into helm and passes `doctor` |
| **M4** | Native clients | `helm-hecate`, `helm-odin`, urania, charon portal | The stopgaps are retired |
| **M5** | helm compositor | `helm-compositor` on Smithay, `NativeBackend` | The ledger runs the screen directly |
| **M6** | Polish | Qt/Kvantum, multi-monitor, a11y, optional minimal motion | Frame budgets held on a 2015-era laptop |

**M3 is the MVP.** M0–M3 is the critical path; nothing in M4+ blocks it.

---

## Sequencing rules

1. **Contracts before implementations.** `helm-core` types land before the crate
   that consumes them, so two components are never invented in parallel.
2. **Stopgaps must be swappable.** Every stopgap (fuzzel, river, btop) sits
   behind a config or a trait, never a hardcoded call. The name `river` may not
   appear outside `crates/helm-session/src/backend/`, `packaging/` and `docs/`.
3. **Nothing merges without a test.** Layout maths gets unit tests; anything
   touching a live socket gets an integration test; anything touching a
   distro gets a CI job on that distro.
4. **The budget is a gate, not a goal.** A change that misses a frame budget in
   [ARCHITECTURE.md §4](ARCHITECTURE.md) is a regression, not a trade-off.
