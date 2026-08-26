# The failure register

Desktop environments fail in a small number of well-known ways. This file lists
them, what helm does about each, and where the guard lives. A new entry is added
whenever we find a new way to break — a bug we fixed but did not write down is a
bug we will ship again.

Legend: **Guard** = the thing that would fail loudly if the mitigation regressed.

---

## Rendering and layout

| Pitfall | What users see | helm's answer | Guard |
|---|---|---|---|
| Rounding loss in tile maths | 1px cracks showing the void between windows, at some resolutions only | Largest-remainder integer partition; rectangles must sum to the workarea exactly | `layout::tests::every_layout_tiles_exactly_for_every_plausible_size` |
| Off-by-one on odd resolutions | The rightmost window is 1px narrow | Same partition; tested at 1281×801 on purpose | same |
| Fractional scaling blur | Everything is soft on a 150% display | Integer geometry; buffers allocated at the output's real scale | *planned:* M2 scale test |
| Focus causes relayout | Windows twitch as you move focus | Focus is a flag on a `Placement`, not an input to geometry | `layout::tests::projection_is_pure_and_focus_only_moves_the_flag` |
| Redraw on a timer | Idle CPU never reaches zero; laptop fans | Event-driven; `HelmState::renders_same_as` drops no-op frames | `state::tests::revision_alone_does_not_force_a_redraw` |

## Being the window manager (river)

| Pitfall | What users see | helm's answer | Guard |
|---|---|---|---|
| A client quantises its proposed size | 4px cracks between tiles that the layout tests cannot see, because the projection was correct and the *client* rounded | Propose at or above the tile and clip with `set_content_clip_box`, so the visible rectangle is exact regardless of what the client does | *planned:* M2 pixel-diff test against a deliberately quantising client |
| `helm-session` stalls | Keys stop responding; river raises `unresponsive`; the session is effectively dead | Nothing on the input path may block — no theme apply, no write to a wedged subscriber. §4 budgets are correctness bounds here | *planned:* M2 watchdog asserting no handler exceeds its budget |
| `helm-session` dies | Windows unplaced, keybindings gone — a sharper failure than a crashed bar | Supervised restart with ledger recovery from the last snapshot | *planned:* M2 |
| Layer-shell not served | **The bar never appears**, and it looks like the bar is broken rather than the WM | helm implements `river-layer-shell-v1`; `doctor` checks it is being served | *planned:* M2 |
| Position submitted during the management phase | Session dies at startup with a protocol error | `set_position` is rendering state and `propose_dimensions` is management state; the backend respects the phase boundary | *planned:* M2 test asserting no `error::sequence_order` across a scripted mutation sweep |
| A stale window manager holds river's global | The supervised `helm-wm` never starts, and a naive restart policy loops forever, burying the message | river answers `unavailable` to a second window-management client; exit 69 plus `RestartPreventExitStatus` stops the loop, and `doctor` names the process holding it | *planned:* M3 |
| Protocol version drift after a river bump | Session fails to start after a routine upgrade | Pin a tested river; version-check at connect and refuse with a clear message rather than misbehaving | *planned:* M2 |

## Fonts and glyphs

| Pitfall | What users see | helm's answer | Guard |
|---|---|---|---|
| Missing Nerd Font | Tofu boxes in the bar on first boot | Glyph inventory + startup probe + documented ASCII fallback for every glyph | `glyphs::tests::a_bare_ascii_font_degrades_instead_of_drawing_tofu` |
| Exotic glyph assumed present | `𓂃` renders as a box in the prompt | Explicit `prompt_sigil_fallback = "~"` in `palette.toml` | same |
| Emoji font hijacks symbols | Runes render in colour, wrong size | Fallback chain is ordered and explicit; no system default | *planned:* M2 render test |

## Colour and theming

| Pitfall | What users see | helm's answer | Guard |
|---|---|---|---|
| Contrast implemented as a filter | Fullscreen GPU pass every frame; accent hues rotate | Contrast derived per-colour in OKLab, capped at the sRGB gamut boundary | `color::tests::contrast_stops_at_the_gamut_boundary_instead_of_desaturating` |
| Unreadable palette after a tweak | Grey-on-grey body text | `Palette::lint` enforces WCAG floors and ≥25° accent separation | `palette::tests::shipped_palette_survives_the_whole_contrast_range` |
| Half-applied theme | Terminal is new, GTK is old, until relogin | Render to temp files, `rename(2)` atomically, then one reload fan-out | *planned:* M1 |
| Colour written down twice | The two drift | Nothing but `palette.toml` may contain a literal colour | *planned:* M1 CI grep |

## Session integration — the classic killers

| Pitfall | What users see | helm's answer | Guard |
|---|---|---|---|
| `WAYLAND_DISPLAY` never reaches D-Bus | File dialogs hang for 25 s, then fail | Session entry imports the environment into systemd *and* D-Bus before starting anything | *planned:* M3 `doctor` check |
| `XDG_CURRENT_DESKTOP` unset | Portals pick the wrong backend, screen share silently fails | Set explicitly to `helm` and exported both ways | *planned:* M3 |
| No portal backend installed | "Open File" does nothing in Firefox | Packages depend on a backend; `doctor` verifies one answers on D-Bus | *planned:* M3 |
| A unit is *skipped* rather than failed | Nothing starts, and `systemctl start` still exits 0 with nothing in `--failed` | An unmet `ConditionEnvironment=` leaves a unit `inactive (dead)` with `ConditionResult=no`. The session entry and `doctor` check each unit's `ActiveState` instead of trusting the exit code | *planned:* M3 |
| The user manager outlives the session | The next login inherits a `WAYLAND_DISPLAY` pointing at a dead socket — symptoms identical to never importing it at all | `systemd --user` and the session bus persist across logout, always when lingering. Teardown clears both after stopping the target, and the entry treats an inherited value as stale | *planned:* M3 |
| Session dies with a client | Bar crashes → black screen | Clients are restartable; the session daemon owns the lifecycle | *planned:* M2 |
| No lock/idle handling | Laptop lid closes, session stays unlocked | Ship an idle+lock unit as part of the session | *planned:* M3 |
| XWayland apps unstyled or scaled wrong | Old apps look broken | XWayland enabled with explicit scaling policy; theming via `Xresources` from the same palette | *planned:* M3 |
| Cursor theme unset | Cursor is X11 default black arrow, or invisible over some surfaces | Cursor theme and size set in the session env and in `gsettings` | *planned:* M3 |

## Packaging

| Pitfall | What users see | helm's answer | Guard |
|---|---|---|---|
| Works on the author's distro only | Install fails on Ubuntu | Three distro jobs in CI, plus a NixOS VM boot test | *planned:* M3 |
| Flatpak apps ignore the theme | One app is bright white | Documented as a *limit*, with the per-app config grant to fix it | ADR 0005 |
| Version skew between components | Bar and session disagree about the protocol | `PROTOCOL_VERSION` handshake; mismatch refuses rather than misreads | `ipc::tests` |
