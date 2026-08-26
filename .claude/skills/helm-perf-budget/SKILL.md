---
name: helm-perf-budget
description: Use before adding a timer, a poll loop, a `sleep`, an animation or transition, a redraw or damage path, a background thread, or any work in a hot path such as layout projection, bar rendering or theme apply. Also use when asked "is this fast enough", "why is idle CPU not zero", "can we animate this", "can we add a blur or rounded corners", or when touching frame budgets, HelmState::renders_same_as, damage tracking, or anything under ARCHITECTURE.md §4.
---

# helm performance budget

"Snappy" is a number here, not an adjective. The budgets in
`docs/ARCHITECTURE.md` §4 are **gates**: a change that misses one is a
regression to be fixed or reverted, not a trade-off to be argued (`docs/MVP.md`,
sequencing rule 4).

**Start in the spec** (S14). Every spec in `docs/specs/` has a **Budgets**
section naming the numbers that component must hold and how they are measured;
`docs/specs/TEMPLATE.md` requires it. A change that alters what a component
does at speed changes that section first — and reference §4 rather than
inventing a new number. ADR 0009 is the decision record for the no-animation
budget.

## The budgets

| Path | Budget | What holds it |
|---|---|---|
| Key press → new geometry submitted | **< 4 ms** | `layout::project` is pure integer arithmetic over a short list |
| State change → bar redraw | **< 8 ms** | damage tracking; `HelmState::renders_same_as` drops no-op frames |
| Bar idle CPU | **~0%** | event-driven modules; only the clock ticks, once a second |
| Cold session start → usable | **< 900 ms** | no GPU context for the bar, no icon-cache scan, no thumbnailer |
| `helm ctl theme apply` | **< 150 ms** | templates rendered in parallel, written atomically |

## Rendering is event-driven, never polled

The one sanctioned timer in the whole desktop is the clock's 1 s tick. Every
other bar module is push-driven (`docs/INTERFACES.md` §3, rule 1); a module that
can only be polled has to justify itself in its own PR, and the justification
has to say why the data source offers no notification.

A poll loop is how idle CPU never reaches zero and how laptop fans start — the
"redraw on a timer" row in `docs/PITFALLS.md`. If you are reaching for an
interval, first check for: a Wayland event, a control-socket `Event::State`, an
inotify watch, a netlink or udev event, or a D-Bus signal.

Even the clock tick must **damage the clock, not the bar**.

## Before you redraw, prove you must

```rust
if state.renders_same_as(&last) { return; }   // no drawing has happened yet
```

`HelmState::renders_same_as` compares everything that is visible and ignores
`revision`, so a module that recomputes to the same string costs nothing. Two
consequences worth internalising:

- Bumping `revision` is not a reason to draw — `state::tests::revision_alone_does_not_force_a_redraw`.
- The same logic upstream: `WmBackend::apply` is called only when the projection
  changed, and must be idempotent (`docs/INTERFACES.md` §1). Because
  `layout::project` is pure, "did anything change?" is an equality check on the
  ledger, not a rectangle diff — that is most of the 4 ms budget.

## Categorically banned in v1

Not "expensive", not "use sparingly" — out (ADR 0009, `design/HANDOFF.md`):

- Compositor-side animations and transitions of any kind.
- Blur, of anything, anywhere.
- Rounded corners. `metrics.radius` is `0` and a non-zero value is refused at
  parse time.
- Shadows, except the 1px border and the **static** inset glow on the focused
  window. Static means it does not fade, pulse or track anything.
- Thumbnails and thumbnailers, including in charon.
- Icon-cache scans at start-up.

The minimal-motion pass is **M6**, opt-in, and bounded to opacity-only fades of
at most 120 ms on overlays — never moving or scaling a window. Until then, a
motionless implementation is the design and not a placeholder.

## Measure, do not guess

There is no benchmark harness in the workspace yet — `helm-core` has no
dev-dependencies, and the CI benchmarks named in §4 are still to be built.
So measure with what exists, and say plainly which number you measured:

```
cargo test --release -p helm-core         # release timings, not debug
/usr/bin/time -v <cmd>                    # wall clock, peak RSS
perf stat -d <cmd>                        # cycles, cache behaviour
```

For a hot path, wrap the *whole* operation in `std::time::Instant` over many
iterations rather than timing one call — at 4 ms the timer noise is comparable
to the work. Remember the dev profile is `opt-level = 1` for the workspace and
`2` for dependencies; a debug-build number is not a budget check.

For idle CPU, watch the process for a minute of genuine idleness
(`top -p <pid>` or `pidstat -p <pid> 1`) rather than sampling once. A 1 s poll
loop looks free in a snapshot.

Guidance for reporting: name the machine, the profile, the number and the
budget. "Faster" is not a measurement.

## Reference

`reference.md` collects the measurement recipes, what each budget is really
guarding against, and the pitfalls register rows that belong to performance.
Open it when you need to actually take a number or are deciding whether a
timer is justified.
