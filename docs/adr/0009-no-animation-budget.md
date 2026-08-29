# ADR 0009 — No animations in v1, and "snappy" is a number

- **Status:** Accepted (ratified 2026-08-28); theme-apply mechanism partially
  superseded by [ADR 0017](0017-immutable-theme-activation-generations.md)
- **Deciders:** helm maintainers
- **Supersedes / Superseded by:** ADR 0017 supersedes only the per-file
  replacement mechanism in the theme-apply budget row; the < 150 ms budget
  remains active and now includes sealed generation publication.

> **Supersession notice (2026-08-29):** The table's per-file replacement text
> is historical. Current apply renders, validates, seals, fsyncs, and selects one
> immutable generation for future launches without reload.

## Context

The handoff is unambiguous: "Snappy: zero animations in v1 (a minimal-motion
pass may come later — design for instant state changes now)." It repeats the
rule in the performance section: "no compositor-side animations, no blur, no
rounded corners, no shadows except the 1px borders + inset glow on focus (a
static box-shadow, cheap)."

The problem with shipping that as written is that "snappy" is an adjective, and
adjectives do not fail a build. Every desktop environment claims to be fast. The
ones that are, are fast because someone measured.

There is also a design argument, not only a performance one. A 150 ms workspace
transition is 150 ms during which the user cannot act on what they asked for. In
a keyboard-first desktop where orbit switching and focus stepping are the two
most frequent operations, animation is latency wearing a costume. The ledger
model (ADR 0001) makes instant state changes cheap: a focus step does not move a
single pixel, so there is nothing to animate even in principle.

## Decision

**v1 has no animations, no transitions, no blur, no shadows and no rounded
corners.** `Palette` enforces the last of these at parse time:
`metrics.radius != 0` is a hard error with the message "helm has no rounded
corners".

Every state change is instant: orbit switch, focus, swap, launcher open and
close, preview toggle, mode change, which-key toggle.

And "snappy" is replaced by five numbers, which are CI gates and not goals.
Standing order S7 and `docs/MVP.md` sequencing rule 4 both say the same thing: a
change that misses a budget is a regression, not a trade-off.

| Path | Budget | How it is held | Gate |
|---|---|---|---|
| Key press to new geometry submitted | < 4 ms | The projection is pure integer arithmetic over a short list; no allocation on the hot path | *planned:* M2 benchmark |
| State change to bar redraw | < 8 ms | Damage-tracked CPU rasterising (ADR 0008); `HelmState::renders_same_as` drops no-op frames | *planned:* M2 benchmark |
| Bar idle CPU | ~0% | Event-driven modules; the clock ticks to the next minute boundary and a shared 1 Hz sampler runs off the input path. Held-key repeat is a separately bounded active-input exception below | *planned:* M2 idle-frame count test |
| Cold session start to usable | < 900 ms | No GPU context, no icon-cache scan, no thumbnailer | *planned:* M2 startup benchmark |
| `helm ctl theme apply` | < 150 ms | Templates rendered serially and replaced atomically per file | *planned:* M1 benchmark |

Benchmarks run on a fixed CI runner class so the numbers are comparable across
commits. M6's acceptance criterion is holding the same budgets on a 2015-era
laptop; until then the runner is the reference.

### Permitted timer exceptions

The shared 1 Hz sampler is required for CPU, memory, GPU, and network-rate
modules; those counters have no useful event source. It runs on one session
thread outside the input path. The clock schedules the next minute boundary,
not a one-second redraw. Neither timer licenses polling by individual clients.

### Held-key repeat

[ADR 0013](0013-river-window-management-backend.md) found that
`river_xkb_binding_v1` sends `stop_repeat`, which establishes that key repeat
for bound keys is the **window manager's** job. If `mod+j` should repeat while
held, `helm-session` repeats it; river does not. That is an active-input timer,
not permission for background polling.

It is scoped so that the idle claim survives intact:

- Armed only on `pressed`, and only for bindings marked repeatable.
- Disarmed on `released` or `stop_repeat`, whichever arrives first.
- Never armed at any other time, so an idle session has no active repeat timer.

An idle session is therefore still idle; a session with a key held down is by
definition not idle. The idle-frame test above is the guard, and ADR 0013 adds
a second guard asserting the timer is disarmed by `stop_repeat` as well as by
`released`.

## The later minimal-motion policy

If motion is ever added — M6 at the earliest, and `docs/MVP.md` lists it as
deferred — it is bound by these rules, taken from the handoff:

1. **Opt-in.** Off by default, enabled by a setting in `palette.toml`.
2. **Opacity only.** No transform, no position, no scale.
3. **Overlays only.** The hecate panel and the scrim may fade. Windows may not.
4. **120 ms ceiling.** Anything longer is latency.
5. **Never move or scale a window.** This is the absolute rule. A window's
   rectangle comes from a pure projection; interpolating towards it would mean
   the screen showing a geometry the ledger never contained.
6. **`prefers-reduced-motion` equivalents win.** If the user's accessibility
   settings ask for reduced motion, the setting is ignored regardless.

Rule 5 is the important one, and it is a consequence of ADR 0001 rather than a
taste judgement.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| **Ship subtle motion from the start** (a 100 ms crossfade on orbit switch) | Motion communicates causality: a fade tells the user *why* the screen changed, which is genuinely useful when six orbits look similar. It is also what every mainstream desktop does, and its absence reads as unfinished to some people | It costs the thing helm is for. In a keyboard-first desktop the orbit switch is a hundred-times-a-day operation, and 100 ms times a hundred is real. It also requires the compositor to hold two frames and interpolate, which conflicts with damage-tracked "only what changed is redrawn" |
| **Make motion a setting from day one** | Users choose; nobody is forced | Doubles the tested surface at the point in the project where we have the least capacity to test it. Both paths need the budgets held, and the animated one needs a frame-pacing story we have not designed. Deferring it to M6 costs nothing, because motion added later is additive |
| **Keep "snappy" as a review standard rather than a CI gate** | Benchmarks in CI are noisy, and a flaky performance gate is worse than no gate; reviewers can catch obvious regressions | Reviewers do not catch a 0.4 ms creep per commit, and that is how desktops become slow. The noise problem is real and is handled by generous absolute thresholds on a fixed runner class, not by abandoning the gate |
| **Budget in frames rather than milliseconds** | Matches how compositors actually schedule; automatically adapts to a 144 Hz display | Only three of the five paths are frame-scheduled at all. Theme apply and cold start are not, and expressing them in frames would be a fiction |

## Consequences

### Good

- Latency is bounded and measured rather than asserted. A regression is a failed
  build, with a number attached.
- The design and the performance rules agree: no shadows and no rounded corners
  is both what the handoff asks for and what the rasteriser is good at.
- Battery life and fan noise benefit from idle actually being idle.
- helm is usable over a remote session and in a VM, where animation is the first
  thing to fall apart.
- No frame-pacing code, no interpolation state, no partially-animated states to
  reason about. The screen always shows a state the ledger contained.

### Bad

- Some users will read instant as jarring, particularly on orbit switch where
  the entire screen changes at once with no cue about what happened. The bar's
  active rune is the only signal, and it is 28px wide.
- No motion means no motion-based affordances at all: no hint that a window went
  to another orbit, no indication that something was stowed rather than closed.
  These need to be solved with static design instead, which is harder.
- Performance gates cost CI time and will occasionally be flaky. Someone has to
  own the thresholds.
- "No animations" is a positioning statement as much as a technical one, and it
  will lose helm some users at first glance.

### Neutral

- Under [ADR 0013](0013-river-window-management-backend.md) this rule stops
  depending on anyone else's configuration. helm is the window manager, so
  window positions come from `layout::project` on every render sequence and
  there is no compositor-side motion to disable. ADR 0002's niri backend needed
  a shipped config turning animations off, and that config was itself a thing
  that could regress; the obligation is gone rather than merely satisfied.

## Reversal

Low for the policy, cheap for the implementation. Adding opt-in opacity fades on
overlays touches `helm-bar` and `helm-hecate` only, and the policy above already
specifies the constraints. Estimated a few days once the compositor is ours.

Removing the budgets is a one-line CI change and would be a mistake; they are
the part of this ADR that is doing the work.

The signal to reconsider is repeated, specific user feedback that the orbit
switch is disorienting. The first response should be a static one — a brighter
active rune, a momentary orbit name in the bar — before reaching for motion.

## Guard

- `palette::tests::out_of_range_values_are_rejected_at_parse_time` — includes
  the `metrics.radius != 0` rejection, so "no rounded corners" is enforced by
  the type system's front door rather than by review.
- `state::tests::revision_alone_does_not_force_a_redraw` — the no-op frame drop
  that keeps idle CPU at zero.
- `layout::tests::projection_is_pure_and_focus_only_moves_the_flag` — the
  structural reason nothing needs animating on focus.
- *Planned (M1–M2):* the five benchmarks in the table above, each failing the
  build on regression past its threshold.
- *Planned (M2):* an integration assertion that a ledger mutation produces
  exactly one render sequence and one frame, with no intermediate geometry —
  the structural form of "no animation" now that helm sets positions itself.
