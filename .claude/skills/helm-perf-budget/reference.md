# helm-perf-budget reference

Measurement recipes and the reasoning behind each budget. The rules and the
banned list live in `SKILL.md`.

## Contents

- [What each budget is guarding](#what-each-budget-is-guarding)
- [Deciding whether a timer is justified](#deciding-whether-a-timer-is-justified)
- [Measurement recipes](#measurement-recipes)
- [Performance rows of the failure register](#performance-rows-of-the-failure-register)

## What each budget is guarding

| Budget | The user-visible failure it prevents | The design property that makes it cheap |
|---|---|---|
| Key press → geometry < 4 ms | The desktop feels laggy exactly when you are navigating fastest | `layout::project` is pure integer maths over a `Vec<WinId>`, with no allocation-heavy bookkeeping and no I/O (ADR 0001) |
| State change → bar redraw < 8 ms | The bar lags behind the thing it is describing | `HelmState` is a plain data struct; `renders_same_as` gates the frame before any drawing |
| Idle CPU ~0% | Fans spin on a laptop doing nothing; battery life drops for no benefit | event-driven modules, one shared 1 Hz sampler for interval-derived rates, minute-aligned clock, damage-tracked repaint |
| Cold start < 900 ms | Login feels like a workstation booting | no GPU context for a 32px bar (ADR 0008), no icon-cache scan, no thumbnailer |
| `theme apply` < 150 ms | A colour tweak feels like a recompile, so nobody tweaks colours | one captured input set is rendered, fully validated, sealed, made durable, and selected as an immutable generation for future launches, with no pointer-switch reload (SPEC 0011, ADR 0017) |

SPEC 0011 supersedes ADR 0009's historical per-file replacement and reload
fan-out for this row. Identical inputs may publish a new generation; the budget
does not create a supported equality/no-op shortcut or weaken full-generation
validation.

## Deciding whether a timer is justified

Work down this list. Stop at the first that applies:

1. **Is there an event?** Wayland event, control-socket `Event::State`, D-Bus
   signal, netlink (network), udev (battery, backlight), inotify (config files).
   If yes, use it — that is the answer.
2. **Can the producer be asked to notify?** A helm-owned component pushing is
   always preferable to a consumer polling.
3. **Is the data genuinely only samplable?** CPU and memory utilisation are the
   honest cases: they are ratios over an interval and have no event. Sample at
   the coarsest interval that still reads correctly, and damage only that
   module.
4. **Otherwise, do not add the timer.** Say what you would have polled and why
   it is not worth the idle cost, in the PR.

`sysinfo` (named in `.claude/memory/20-environment.md`) is the sampling
dependency for the cases that reach step 3.

## Measurement recipes

### A hot path in `helm-core`

```rust
// In a #[test] or a small bin, always in release.
let start = std::time::Instant::now();
for _ in 0..10_000 {
    std::hint::black_box(layout::project(&orbit, area, params));
}
let each = start.elapsed() / 10_000;
```

Run with `cargo test --release -p helm-core -- --nocapture`. `black_box` matters:
`project()` is pure and the optimiser is entitled to delete an unused result.

### A whole command

```sh
/usr/bin/time -v helm ctl theme apply     # wall clock and peak RSS
perf stat -d helm ctl theme apply         # cycles, instructions, cache misses
strace -c -f helm ctl theme apply         # syscall counts, when I/O is suspected
```

### Idle CPU

```sh
pidstat -p "$(pgrep -x helm-bar)" 1 60    # a minute of genuine idleness
```

Sampling once tells you nothing: a 1 s poll loop is invisible in a snapshot and
obvious over a minute.

### Start-up

```sh
systemd-analyze --user critical-chain helm-session.target
systemd-analyze --user blame | head
```

Both are M3-and-later, once the units exist. Until then, time the binary
directly and note that it excludes session set-up.

### Caveats to state whenever you quote a number

- Debug versus release. The dev profile is `opt-level = 1` for the workspace and
  `2` for dependencies — good for iteration, not a budget check.
- The agent container has no Wayland display, no GPU and no D-Bus session
  (`.claude/memory/20-environment.md`), so any budget involving drawing or the
  session must be measured in CI or on hardware. If it cannot be measured here,
  say so rather than extrapolating.
- The budgets are meant to hold on a 2015-era laptop (the M6 "done when"), not
  on a fast development machine.

## Performance rows of the failure register

From `docs/PITFALLS.md` — each names the guard that fails if the mitigation
regresses:

| Pitfall | Guard |
|---|---|
| Redraw on a timer; idle CPU never reaches zero | `state::tests::revision_alone_does_not_force_a_redraw` |
| Focus causes relayout; windows twitch as you navigate | `layout::tests::projection_is_pure_and_focus_only_moves_the_flag` |
| Rounding loss in tile maths | `layout::tests::every_layout_tiles_exactly_for_every_plausible_size` |
| Contrast implemented as a filter — a fullscreen GPU pass every frame | `color::tests::contrast_stops_at_the_gamut_boundary_instead_of_desaturating` |
| A crashed bar taking the session with it | clients are restartable units; see the `wayland-session-integration` skill |

A new performance failure mode is a new row there, with its guard named. A bug
fixed but not written down is a bug that ships again.
