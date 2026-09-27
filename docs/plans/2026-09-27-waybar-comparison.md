# Bounded Waybar comparison implementation plan

> Execution: use the executing-plans skill for one implementation pass and at
> most one focused correction. This document authorizes no production switch.

**Goal:** establish whether packaged Waybar offers a cheaper path to the required
bar, without blocking launch or rebuilding working commodity metrics.

**Architecture:** a CI-only Python adapter subscribes to Realm's existing socket
and emits one custom-module JSON line per changed display value. Packaged Waybar
supplies clock, CPU, memory, network and battery. The first experiment keeps
realm-bar for key discovery, making the temporary hybrid and its duplicate
costs explicit rather than claiming a completed replacement.

**Stack:** pinned Nixpkgs Waybar, existing River session VM, Python standard
library, existing Realm protocol v2. No browser automation or product API.

**Specs:** [SPEC0030 B1](../specs/0030-reuse-first-session-theme-mvp.md),
[SPEC0031](../specs/0031-mvp-launch-delivery.md). Packaging and VM execution stay
CI-only; local work is source tests. One pass plus one correction is the limit.

## Verified interfaces and constraints

- [Upstream custom-module manual](https://github.com/Alexays/Waybar/blob/master/man/waybar-custom.5.scd)
  documents `exec`, `return-type: json` and continuous output when neither
  `interval` nor `signal` is configured. Emit one flushed JSON object per line
  on the child's stdout; Waybar is not a general stdin consumer. Disable custom
  click/scroll commands and use `escape: true` for window-title markup.
- `crates/realm-control/src/client.rs:159` sends Subscribe and expects an initial
  State event. `crates/realm-core/src/ipc.rs` defines subsequent State/Shutdown
  events. Adapter sends Hello v2, checks reply/version, sends Subscribe, then
  reads newline frames; no GetState polling and no action injection. Validate
  against the packaged Waybar version's shipped manual during CI, not only
  upstream master. Record exact package versions and source revision.
- `realm-bar/src/model.rs` creates its top bar unconditionally alongside the
  which-key and grimoire surfaces. There is no existing overlay-only switch.
  Do not invent one merely for this experiment. Put candidate Waybar at the
  bottom and retain realm-bar temporarily; identify duplicate UI, exclusive
  zones and process costs in every result. This hybrid cannot by itself prove
  that replacing realm-bar preserves its help surfaces or reduces total cost.
- Reuse `packaging/native-vm/window_roundtrip.py::exercise_controls` for real
  keyboard transitions and `packaging/nix/checks.nix` screenshots/state evidence.
  Use existing module fixtures for battery/no-battery semantics, not a fictitious
  battery in the VM. Actual battery hardware coverage remains separate.

## Task 1: event-fed adapter and CI-only candidate

Files: create `packaging/nix/waybar_compare.py` and
`packaging/nix/test_waybar_compare.py`; modify `packaging/nix/checks.nix` only
for a separate opt-in comparison test/configuration. Keep normal session-boots
acceptance and production packages unchanged. Add the check output through the
existing `flake.nix` check wiring only if required; never make its experiment
failure mask ordinary acceptance evidence.

- [ ] Write failing source tests for `render_state(state: dict) -> dict`:
  all six numbered orbits with occupied/active/urgent distinctions, layout,
  mode, focused title, chord echo; empty title and markup-like title roundtrip
  as literal JSON text. Include narrow/long-title input without unbounded text.
- [ ] Write failing socket fixtures for Hello/initial State/update/Shutdown,
  malformed/version-mismatch frames, EOF and a frame beyond Realm's existing
  maximum; verify no polling requests and unchanged rendered output suppressed.
- [ ] Implement one blocking subscription, flush each changed JSON line, emit
  diagnostics to stderr and exit nonzero on broken protocol. No reconnect
  framework; the bounded VM test treats unexpected adapter exit as failure.
- [ ] Add CI-only Waybar JSON configuration using one `custom/realm` module and
  upstream clock/cpu/memory/network/battery. Use one-second sampled metrics,
  no animation, palette-derived CSS from the selected login palette, and no
  literal copied color palette. Continuous custom module has no interval.
- [ ] Run Python tests red then green; check Nix syntax without building.
  Commit source/spec/test together, then invoke existing CI packaging path.

## Task 2: one bounded comparison run

In the same pinned VM, same login selection, output mode and window workload,
run baseline realm-bar then the temporary hybrid. Waybar is a user process in
the existing session, not a second session supervisor. Retain structured results
and screenshots under `waybar-comparison/` and upload through the existing
artifact mechanism. Always terminate candidate and adapter and verify baseline
help/focus still works; failed cleanup is a recorded failure.

- [ ] Source-test configuration/adapter wiring before adding the VM sequence.
- [ ] Run real keyboard focus/swap/orbit-return/mono-triptych transitions.
  Correlate ledger/State values, adapter output and inspected screenshots:
  six orbit states, layout, mode, title and chord must match Realm, never River
  tag assumptions. Exercise long/non-ASCII titles and actual font fallback.
- [ ] Open/dismiss which-key and full help; verify neither steals keyboard
  focus. Inspect usable workarea with both exclusive zones; fullscreen/unfullscreen
  must preserve client coverage and restore bars. Use the shipped keymap, not
  invented chords. Capture at configured and one additional supported scale.
- [ ] Observe real clock and sampled CPU/memory/network modules. Record battery
  absent in this VM; battery-present/low-state unit fixtures are not hardware
  evidence. Never mark unobserved battery UI behavior passed.
- [ ] Use host monotonic timestamps for process launch to first screenshot with
  expected text: three trials per variant, each capped at 30 seconds. Label this
  **probe-observed launch-to-visible upper bound**, not machine cold boot or
  compositor presentation latency. For three state changes, record key dispatch,
  socket observation and first matching screenshot; separately report adapter
  receive-to-flush timing. OCR/screenshot cadence is part of each interval.
- [ ] After 10 seconds settling, take a 30-second idle sample per variant using
  process CPU tick deltas and RSS at one-second intervals. Include Waybar,
  adapter, realm-bar and WM identities, aggregate CPU seconds and per-process
  RSS; report RSS shared-page double counting and VM noise. Preserve baseline
  samples and raw values, not just a claimed percentage improvement.
- [ ] Compare with existing ARCHITECTURE targets but report **not measured** for
  true input-to-present latency, cold boot, wakeups not instrumented, or physical
  battery behavior. A timestamped screenshot cannot establish sub-frame latency.

## Decision and stop rule

Record functional misses, dependency/process overhead, owned adapter code and
the additional work required to remove duplicated bar responsibilities. This
plan's hybrid preserves discovery but does not establish a cheaper complete
replacement: do not subtract the retained bar's measured cost from totals or
claim overlay-only behavior that does not exist. Permit one correction only for
a concrete observed blocker. If a complete cheaper route is not demonstrated,
retain realm-bar for launch and record the reasons; do not grow an overlay API,
second renderer, generic benchmark harness or lifecycle manager to rescue it.

Review focus: malformed/disconnected stream, markup/title fallback, absent
battery, focus/exclusive-zone/fullscreen interaction, and misleading measurement
precision are covered by the tasks above. No unresolved product decision blocks
running the experiment; adopting a replacement still requires B1 evidence and
a recorded decision. This plan is not that evidence.
