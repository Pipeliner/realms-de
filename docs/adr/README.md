# Architecture decision records

Every decision in [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md) §3 has a record
here, with its alternatives argued fairly and its reversal cost stated honestly.

The rule (from [`CLAUDE.md`](../../CLAUDE.md)): **a decision that turns out
wrong gets a new ADR, not a quiet edit.** Superseding is normal and healthy;
rewriting history is not. Standing order S14 puts these ahead of code — nothing
non-trivial is implemented before the decision behind it is written down.

Every ADR carries a **Guard**: the test, lint or CI job that fails if the
decision silently stops being true. A decision with no guard is a preference.

Copy [`template.md`](template.md) to start a new one.

## The record

| # | Title | Status | In one line | Reversal |
|---|---|---|---|---|
| [0001](0001-ledger-as-single-source-of-truth.md) | The ledger is the single source of truth | Accepted | Ordered `Vec<WinId>` per orbit; layouts are pure projections; undo restores an older ledger | **Structural** — the whole DE assumes it |
| [0002](0002-borrow-a-compositor-first.md) | Borrow a compositor first; write ours later | ~~Superseded~~ by [0013](0013-river-window-management-backend.md) | Ship on niri behind a `WmBackend` trait. Kept in full: its mapping table is the evidence for the move | — |
| [0003](0003-session-daemon-owns-state.md) | A session daemon owns the state | Accepted — theme-reload consequence superseded by [0017](0017-immutable-theme-activation-generations.md) | `helm-session` holds `HelmState`; clients subscribe and hold nothing | Low |
| [0004](0004-ndjson-control-socket.md) | Newline-delimited JSON over a unix socket | Accepted | One JSON value per line at `$XDG_RUNTIME_DIR/helm/ctl.sock`, with a version handshake | Low — a D-Bus surface would be additive |
| [0005](0005-palette-toml-single-source.md) | One `palette.toml`, everything generated | Accepted — activation clauses partially superseded by [0017](0017-immutable-theme-activation-generations.md) | No colour is written down twice; derive, lint, and render from one palette | Low per target |
| [0006](0006-oklab-contrast-not-filters.md) | Contrast derived in OKLab, gamut-capped | Accepted | A `contrast()` filter costs a fullscreen pass per frame and rotates accent hues | Low |
| [0007](0007-reuse-yazi-btop-starship.md) | Reuse yazi, btop and zsh+starship | Accepted | charon and horus are ~90% theme and keymap; always behind a seam | Low per tool |
| [0008](0008-layer-shell-rendering-stack.md) | `smithay-client-toolkit` + `tiny-skia` + `cosmic-text` | Accepted | Pure Rust, no GPU context for a 32px bar, real font fallback | **Medium** — a client rewrite |
| [0009](0009-no-animation-budget.md) | No animations; the frame budget is a CI gate | Accepted — theme-apply mechanism partially superseded by [0017](0017-immutable-theme-activation-generations.md) | "Snappy" is five numbers, not an adjective | Low |
| [0010](0010-nix-flake-as-reference-build.md) | The Nix flake is the reference build | Accepted — Fedora baseline clauses partially superseded by [0015](0015-fedora-44-pre-alpha-baseline.md) | root flake plus tracked native distro packaging; a NixOS VM test boots the session | **Medium** |
| [0011](0011-session-integration-contract.md) | The session entry owns the environment handshake | Accepted | Import into systemd **and** D-Bus before starting anything, or portals hang | Low — the requirement is not reversible |
| [0012](0012-font-fallback-is-a-contract.md) | Font fallback is a contract | Accepted — guard pending | Glyph inventory, a startup probe, and an ASCII fallback for all 37 glyphs | Low |
| [0013](0013-river-window-management-backend.md) | helm **is** the window manager, on river's protocol | Accepted — supersedes [0002](0002-borrow-a-compositor-first.md); Fedora part of Decision 4 superseded by [0015](0015-fedora-44-pre-alpha-baseline.md) | river 0.4 moved window management out of the compositor; every row 0002 marked lossy becomes faithful | **Medium** — back to niri is an architecture change, not a module swap |
| [0014](0014-local-agent-sdd-pilot-governance.md) | Local agent-SDD pilot records are tracked but non-authoritative | Accepted | Metadata-only checkpoints/evidence aid handoff; docs, GitHub and tests retain their existing authority | Low — remove pilot records and validator |
| [0015](0015-fedora-44-pre-alpha-baseline.md) | Fedora 44 is the sole explicit pre-alpha Fedora baseline | Accepted — partially supersedes [0010](0010-nix-flake-as-reference-build.md) and [0013](0013-river-window-management-backend.md) for Fedora | One exact Fedora release and native River candidate, without implying RPM/session support | Low — accept a successor baseline and replace the single lane |
| [0016](0016-packaged-helm-sdd-carries-git.md) | The Nix-installed `helm-sdd` carries Git in its own runtime closure | Accepted | Git is wrapped only for the local Git-backed validator, never the desktop session | Low — replace with a separately specified interface |
| [0017](0017-immutable-theme-activation-generations.md) | Theme activation uses sealed immutable generations | Accepted — partially supersedes [0005](0005-palette-toml-single-source.md) | Launches pin a digest-bound sealed tree; pointer commits change future launches only and never reload | Medium — changes #22/#132 launcher and lifecycle seams |
| [0018](0018-fresh-desktop-exec-only.md) | Desktop launch is fresh-process Exec only | Accepted | Reject D-Bus activation before side effects; plain Exec is a fresh child-only immutable plan | Medium — a future D-Bus path is additive only |
| [0019](0019-nix-ci-cache-is-optional.md) | Nix CI cache is optional | Accepted | External cache authentication may not prevent the reference build or VM evidence | Low — add only reviewed non-blocking cache support |

All active decisions through 0013 were **ratified by the owner on 2026-08-28**,
after the correction set tracked by #3. ADR 0015 was authorized by the owner
after independent review on 2026-08-29. "Accepted" means we are building on it, not that it is settled forever —
0002 is the worked example, superseded within a day of being written and kept
intact because the trail is the point.

## Open questions marked `needs-human`

Standing order S3: anything needing judgement the repository cannot supply is
named, with its options and a recommendation, rather than guessed at.

| ADR | Question | Recommendation |
|---|---|---|
| [0013](0013-river-window-management-backend.md) | `river-window-management-v1` is declared stable with a compatibility pledge, but river is pre-1.0 and its last release cycle was extremely breaking. What do we do if the pledge does not hold? | Pin and follow, with an accelerated `helm-compositor` as the standing mitigation |
| [0013](0013-river-window-management-backend.md) | Confirm whether Ubuntu 24.04 supplies a River release compatible with `river-window-management-v1`. The remaining Ubuntu decision to vendor a pinned 0.4.x rests on this | Verify in Ubuntu packaging CI before packaging work starts |
| [0010](0010-nix-flake-as-reference-build.md) | Where do the `.deb` and `.rpm` actually live: GitHub Releases, a self-hosted apt repo plus Copr, or official distro repositories? Who holds the signing key? | Releases for M3; a repo and Copr once there are users to upgrade |
| [0011](0011-session-integration-contract.md) | Which lock screen ships? `ext-session-lock-v1` is now a hard requirement, so the real choice is waylock (small attack surface, colour-only theming) versus gtklock (looks like helm, drags GTK into the lock path). SPEC 0005 and `docs/integration/session-services.md` disagree | waylock, with the fidelity loss recorded in ADR 0005's limits table |
| [0011](0011-session-integration-contract.md) | Idle policy defaults: blank timeout, lock timeout, and whether lid-close locks unconditionally | Not to be guessed; these are user-visible security defaults |

### Resolved

| ADR | Question | Resolution |
|---|---|---|
| ~~[0002](0002-borrow-a-compositor-first.md)~~ | niri cannot express triptych, even or stow — ship the approximation, restrict layouts, or reopen river? | **Resolved by [0013](0013-river-window-management-backend.md).** river was reopened for a stronger reason than the one 0002 considered; no approximation is needed |
| ~~[0002](0002-borrow-a-compositor-first.md)~~ | Does stow map to a hidden niri workspace, or refuse `mod+s`? | **Resolved by [0013](0013-river-window-management-backend.md).** `river_window_v1::hide`/`show` maps stow exactly, and with matching semantics |
| [0013](0013-river-window-management-backend.md) | Does Fedora require a vendored River because its supported package lacks `river-window-management-v1`? | **Resolved for the Fedora 44 pre-alpha baseline by [0015](0015-fedora-44-pre-alpha-baseline.md).** Fedora's official repositories exposed `river-0.4.8-1.fc44` during the 2026-08-29 review; runtime compatibility remains an M2 test obligation |

## Guards at a glance

The tests that currently exist in `crates/helm-core` and would fail if a
decision silently stopped being true:

| ADR | Guard |
|---|---|
| 0001 | `layout::tests::projection_is_pure_and_focus_only_moves_the_flag`, `layout::tests::every_layout_tiles_exactly_for_every_plausible_size`, `ledger::tests::undo_restores_the_exact_previous_ledger` |
| 0003 | `state::tests::revision_alone_does_not_force_a_redraw`, `state::tests::state_round_trips_through_json` |
| 0004 | `ipc::tests::requests_round_trip_through_a_frame`, `ipc::tests::unknown_frames_are_an_error_not_a_panic`; required M2 transport guards are named in ADR 0004 and SPEC 0003 |
| 0005 | `palette::tests::shipped_palette_passes_its_own_lint`, `palette::tests::lint_catches_muddy_text_and_duplicate_accents` |
| 0006 | `color::tests::contrast_stops_at_the_gamut_boundary_instead_of_desaturating`, `color::tests::contrast_preserves_accent_hue` |
| 0009 | `palette::tests::out_of_range_values_are_rejected_at_parse_time` (enforces `metrics.radius == 0`) |
| 0012 | `glyphs::tests::a_bare_ascii_font_degrades_instead_of_drawing_tofu`; ratification also requires `palette::tests::empty_typography_fallback_is_rejected` to fail for an empty-only fixture and pass for the shipped palette |
| 0017 | `generation::tests::g1_selected_old_generation_keeps_descriptor_pinned_bytes_after_new_commit`, `generation::tests::g5_invalid_current_refuses_without_creating_a_lease`, `generation::tests::g8_recovery_fails_closed_for_corrupt_missing_mismatched_and_special_pointers` |

ADRs 0007, 0008, 0010, 0011 and 0013 depend on guards that land with their
milestones; each names them as *planned* with the milestone attached. 0013's
seam guard supersedes 0002's: `river` must appear nowhere outside the backend
module, packaging and docs, and `niri` nowhere outside docs.
