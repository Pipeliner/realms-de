# ADR 0021 — Bound Wayland ingress and dispatch per service quantum

- **Status:** Accepted (2026-09-11)
- **Deciders:** realm maintainers, repo owner
- **Supersedes / Superseded by:** Supplements [ADR 0013](0013-river-window-management-backend.md) and [SPEC 0003](../specs/0003-realm-session.md).

## Context

SPEC 0003 makes Realm's key-to-finish latency a correctness budget and requires
one bounded, nonblocking backend service quantum. The normal
`wayland-client` 0.31 event path cannot establish that bound:
`EventQueue::dispatch_pending` dispatches every queued event, and the default
pure-Rust backend read path continues receiving and parsing until the socket
would block. Under continuous compositor output, either operation can perform
work proportional to an adversarial queue rather than to one declared quantum.

The reference libwayland API added
`wl_display_dispatch_queue_pending_single`, which dispatches at most one queued
event, in version 1.25. Realm cannot use that symbol as its portability answer:
the current Rust client API does not expose it, and the supported distro
baseline cannot be silently raised to a system libwayland version merely to
obtain it. That function also bounds dispatch only; it does not by itself bound
bytes or messages admitted during a readable-fd service quantum.

The ordinary prepare-read contract remains mandatory. A successful preparation
must be consumed exactly once by reading or cancelled, queued events must be
dispatched before preparation can succeed, and a blocked flush must request
writable readiness rather than spin.

The [Wayland wire format](https://wayland.freedesktop.org/docs/book/Protocol.html#wire-format)
gives message size 16 bits, and descriptors arrive as ancillary `SCM_RIGHTS`
data that may be attached to any byte. Partial messages must therefore be
retained rather than requiring one receive to contain a whole event. Linux
[`unix(7)`](https://man7.org/linux/man-pages/man7/unix.7.html) bounds one
`SCM_RIGHTS` array at 253 descriptors; truncating ancillary data would destroy
the message-to-descriptor association and is fatal.

## Decision

1. Preserve the hard SPEC 0003 bound. A River backend service invocation selects
   exactly one phase: dequeue one normalized public event, dispatch at most one
   queued protocol event, admit one bounded unit of readable ingress, attempt
   one flush, or attempt one read preparation.
2. Define one ingress unit as exactly one nonblocking `recvmsg` attempt into a
   16,384-byte buffer with ancillary capacity for 253 file descriptors. It makes
   no retry/drain loop. It admits at most the returned bytes/descriptors into an
   incremental decoder, retains partial frames for later quanta, and ends the
   quantum. `WouldBlock` after readable readiness is progress; EOF is disconnect;
   `MSG_TRUNC`, `MSG_CTRUNC`, invalid framing, or excess retained-frame storage is
   fatal. Retained undecoded bytes are capped at 65,535, the largest value the
   wire header can encode. Across the partial decoder, queued protocol messages,
   and normalized-event staging, the backend may own at most 253 received file
   descriptors in total. A receive that would cross that total closes every
   descriptor received by that call plus every descriptor retained by the
   failed ingress state exactly once, then fails fatally. Per-call ancillary
   capacity is not treated as a global bound.
3. Do not call stock unbounded `dispatch_pending`, `dispatch`, `roundtrip`, or
   drain-until-`WouldBlock` read paths from `RiverBackend::service`.
4. Supply the missing primitives through the smallest audited Wayland client
   dependency adaptation that bounds both event dispatch and readable ingress.
   Pin its exact source and checksum through Realm's existing retained-source and
   offline CI process. The adaptation must remain private to the River backend;
   it is not a second event loop or a public Realm framework.
5. Keep the prepare/read/cancel guard in backend-owned state across the outer
   poll boundary. No dispatch, roundtrip, or second preparation may occur while
   that guard is live. `WouldBlock` after a valid read is progress, not failure.
6. Queue `OperationCompleted` only after the flush containing every required
   `manage_finish` and `render_finish` has succeeded. A flush returning
   `WouldBlock` enables `POLLOUT` and ends that quantum. After completion, an
   operation may be reported synchronously `Complete` only when the admitted
   post-turn epoch is already sealed empty. Any decoder fragment or queued
   protocol message admitted before that decision forces `Pending` even if the
   final flush succeeded; emit matching Ok first, then normalize that epoch
   into one tagged turn or empty marker. An empty retained-observation marker
   is eligible only after every decoder
   fragment and queued protocol message admitted before that completion has
   been normalized. Facts still unread in the kernel are outside that admitted
   epoch and may form a later ordinary turn; no compositor-wide sync roundtrip
   is required.
7. Package compilation and dependency verification remain CI-only. This ADR
   does not authorize installing local distro packaging tools.
8. Bound normalized semantic work as well as raw I/O. The MVP admits at most
   256 managed windows, 64 configured bindings, and 256 ordered policy events
   per live/draining River response boundary. A separate 256-item staged-effect
   cap applies across the complete Session transaction and every tagged
   follow-up; an external admitted effect counts with all derived effects.
   Every exposed turn, including replay, carries at most 65,535 total UTF-8
   bytes across all String occurrences. Modifier-change old/new vectors are
   canonical enum-ordered duplicate-free subsets of the four accepted modifiers.
   Backend-local River state is independently capped at 16 live outputs, 16
   live seats, 64 live input devices, and 64 live libinput devices, including
   pre-`done` objects. Each object kind uses a checked nonzero never-reused
   `u64` creation ordinal. Cap/ordinal overflow is fatal before installing map
   state or dependent children and destroys newly received/owned objects once;
   removal frees a live slot but not the ordinal. Unused input names and
   libinput arrays are discarded in their one bounded dispatch quantum rather
   than retained.
   River enforces these bounds before exposure and Session independently checks
   them before partial reduction.
   Initial replay may contain at most 259 events: 256 opens, optional ordinary
   focus, optional exclusive focus, and the final barrier event. It contains no
   workarea: River creates the output objects needed by layer shell inside that
   replay. Realm answers it without projection or enabled bindings, then gates
   Live/readiness on a later selected-output workarea turn and its successfully
   finalized complete projection. Coalescing is legal only when it
   preserves exact report-order reduction. The sole named exception is a
   backend-local object opened and closed before either fact is exposed; it is
   defined never to enter Realm identity authority. Once an open is exposed,
   watermark reduction must be preserved. Binding press/release/repeat-stop,
   unbound-key, process, and quit actions are never coalesced or reordered. A
   first Quit suppresses later derived actions/effects across the whole
   transaction, not authoritative facts. Response close edges are unique live
   Realm ids in first-occurrence order. A limit violation is fatal before a
   partial `PolicyTurn` is exposed or answered. The full
   worst-case turn and response must satisfy SPEC 0003's measured key-to-finish
   budget; #65 may lower these constants if the real bound fails, but no
   implementation may silently raise them.

Phase selection is deterministic. Choose the first eligible item in this order:

1. attempt one pending output flush when it has not yet blocked, or when the
   current poll result newly supplies writable readiness;
2. dequeue one normalized public event;
3. dispatch one queued protocol event, only when no read guard is live;
4. consume the one live read guard with the bounded `recvmsg` when the current
   poll result supplies readable, hangup, or error readiness;
5. attempt one read preparation when no guard and no queued work exists.

After the exit cutoff, phases 2 through 5 are permanently ineligible: there is
no public dequeue, protocol dispatch, ingress, or read preparation. Phase 1 may
perform only finite output already ordered by the cutoff: immutable bytes and
the finish for any accepted response phase already open/parsed, a neutral
finish for an unexposed sequence, and the exit request. A later render phase
whose start remains kernel-unread is abandoned. Post-cutoff
`poll_interest.immediate` therefore means only that finite internal/flush work;
readable-only readiness performs no work, while
independent terminal readiness completes the backend only after the final
flush.

No-guard read preparation is itself immediate work: `poll_interest.immediate`
is true whenever phase 5 is eligible, so the outer loop cannot block without a
prepared guard. A successful preparation clears that cause. Preparation that
finds queued work or returns no guard is progress and leaves another immediate
quantum eligible. `POLLNVAL` is an immediate fatal backend-incarnation error;
it is never mapped to ordinary readable readiness. `POLLIN` maps to readable;
`POLLERR` and `POLLHUP` map independently to terminal so a live guard is
consumed. After the exit cutoff, readable interest is false while terminal
observation remains active; unread payload therefore cannot hot-loop or be
mistaken for the expected disconnect.

A flush that returns `WouldBlock` is not immediately eligible again: it must
wait for a later poll result carrying writable readiness. If readable and
writable are simultaneously ready, the flush wins that quantum; its finite
buffered output cannot be starved by continuous peer input. If a blocked flush
is not currently writable, an eligible read phase may proceed. After every
phase the caller discards consumed readiness, recomputes interest, and obtains a
fresh poll result before treating the same readiness edge as permission again.

## Alternatives considered

| Option | Why it was attractive | Why it lost |
|---|---|---|
| Accept one stock `wayland-client` batch as a quantum | No dependency adaptation and idiomatic public API use | The batch has no semantic work bound, so it cannot prove the accepted liveness contract under adversarial input. |
| Require system libwayland 1.25 and call its single-event dispatcher | Upstream C implementation already bounds dispatch | It raises the platform requirement, is not exposed by the current Rust API, and still leaves readable ingress unbounded. |
| Move Wayland onto a worker thread | Keeps the main loop superficially responsive | Sequence responses and input policy must remain linear with Session state; an unbounded worker queue only moves the starvation and adds synchronization. |
| Maintain a minimal pinned client adaptation | Makes both dispatch and ingress bounds explicit while retaining the existing architecture | Chosen despite maintenance cost because it is the only option that proves the hard bound on current targets. |

## Consequences

### Good

- One backend service call has a testable maximum semantic workload.
- A flood of compositor events cannot monopolize the control, timer, or worker
  arbitration loop.
- Prepare-read and nonblocking-flush ownership become explicit rather than
  emergent library behavior.

### Bad

- Realm owns a small compatibility patch until upstream exposes both required
  bounded primitives.
- Wayland client upgrades require checking the adapted internal seam, not only
  compiling against a new public API.
- Real-socket tests are required; a scripted fake cannot prove the bound.
- The MVP rejects workloads beyond 256 managed windows, 64 bindings, 256 live
  policy events per turn, 256 staged effects per transaction chain, 16 outputs,
  16 seats, 64 input devices, 64 libinput devices, or the explicit 259-event
  replay shape instead of allowing unbounded latency.

### Neutral

- This does not change River's policy-turn semantics or Realm's public IPC.
- The outer poll arbiter remains issue #38; the bounded Wayland implementation
  and protocol sequencing remain issue #40; the real latency measurement
  remains issue #65.

## Reversal

Remove the private dependency adaptation when an upstream Rust API both
dispatches at most one selected event and caps readable ingress per call on all
supported baselines. Reversal is localized to the River backend dependency and
its service state machine; the `WmBackend` contract and outer poll loop remain.

## Guard

Planned for M2 issue #40:
`protocol_dispatch_is_capped_per_service_quantum`,
`read_ingress_is_capped_under_continuous_peer_output`,
`partial_wayland_frame_continues_across_bounded_ingress_quanta`,
`truncated_ancillary_data_is_fatal`,
`retained_descriptor_cap_closes_every_owned_fd_on_overflow`,
`policy_turn_batch_is_capped_without_reordering_inputs`,
`river_policy_text_and_modifier_bounds_precede_exposure`,
`maximum_policy_turn_meets_the_response_budget`,
`writable_finish_flush_preempts_continuous_readable_ingress`,
`prepared_read_is_consumed_or_cancelled_exactly_once`,
`prepare_read_none_forces_immediate_dispatch_before_poll`, and
`completion_does_not_emit_empty_marker_before_precompletion_ingress_is_normalized`,
`synchronous_flush_with_admitted_postturn_work_returns_pending`,
`retained_observations_preserve_equivalent_report_order_under_bounds`,
`retained_observation_capacity_is_fatal_before_partial_exposure`,
`tombstoned_retained_open_emits_one_close_and_no_placement`,
`begin_exit_finishes_partially_flushed_open_phase_without_replacement`,
`begin_exit_abandons_pending_unread_render_phase`,
`begin_exit_answers_only_unexposed_sequences_with_exit_policy`,
`exit_cutoff_disables_read_interest_and_requires_postflush_terminal`,
`flush_would_block_requests_pollout_without_busy_retry` use a real socket and
fail if one service invocation performs more than its selected phase. Issue #65
owns
`realm_session::tests::key_to_manage_finish_meets_four_millisecond_budget_on_reference_linux`.
That test retains the end-to-end key-press latency guard.
Issue #38 additionally proves `idle_backend_prepares_read_before_poll`,
`backend_hup_only_maps_to_terminal_readiness`, and
`backend_pollnval_is_fatal_without_service` at the outer-loop boundary.

## Needs a human

No further decision is required. The owner authorized MVP decisions that can be
made from repository and primary-source evidence; weakening an accepted
user-visible liveness guarantee would require a new explicit decision.
