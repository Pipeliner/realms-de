# Session policy-turn transactions implementation plan

> **For agentic workers:** use `superpowers:subagent-driven-development` and
> `superpowers:test-driven-development` task by task.

**Goal:** Deliver the compositor-independent policy-turn transaction and
poll-service prerequisite for issue #38. Session must answer each River policy
boundary exactly once and must not commit, publish, reply, spawn, or quit before
the backend response reaches its final clean boundary.

**Scope:** This is a mergeable library slice plus the minimal realm-control
response-receipt prerequisite. It does not implement River
protocol objects, Wayland ingress/dispatch, the daemon poll set, listener
activation, socket `ConnectionId` completion routing, or real latency evidence.
Those remain #38/#40/#65 work.

**Accepted contract:** `docs/specs/0001-realm-core-contracts.md`,
`docs/specs/0003-realm-session.md`,
`docs/specs/0007-control-socket-security.md`, accepted sections 1 and 4 of
`docs/INTERFACES.md`, and ADR 0021.

## Invariants

- A policy turn is one ordered batch plus one mandatory response. No independent
  projection, close, focus, or binding-policy submission method exists.
- Session alone maps binding ids to `Binding`, `Action`, and `Mode`.
- Clean projection-equal actions that need no binding/edge response finalize
  locally. Every offered policy turn still receives a fresh-ticket response.
- External desired work that needs a response, and every close with a target,
  first reserves its origin ticket, calls error-atomic `request_policy_turn`,
  and returns only `pending_action` on request success. Initial request
  `Io`/`Unsupported` returns an immediate Error, but the reserved ticket number
  remains consumed and is never reused.
- `respond_policy_turn` errors are fatal. `Pending` terminal failures use the
  closed component-precedence table in SPEC 0003.
- A matching successful terminal result is not a commit. The final boundary is
  either a clean empty marker or `Complete` from the final tagged-turn/repair
  response.
- A dirty empty marker requests a repair turn; it cannot answer a turn that does
  not exist.
- Desired/key deltas roll back on nonfatal terminal failure. In a desired,
  close, or key/effect-mixed transaction all compositor facts remain private W
  until finalization. An observed-only batch instead promotes every fact to
  in-memory C when its response safely finishes and exposes only the resulting
  closed-V1 persistable consequences through `snapshot()`, while V/L wait for
  the final clean boundary. Process/quit effects are suppressed on failure.
- Repair dirtiness follows the attempted-component predicate, not a
  changed-value predicate: `projection: Some(_)` is dirty even when equal, and
  bindings are dirty when enabled/watch values differed, their cache was
  already dirty, or any next-key edge was emitted.
- Next-key policy is `Preserve | Ensure | Cancel`; uncertain failed Ensure is
  repaired with Cancel and is never replayed.
- Close target is fixed at admission and its edge is one-shot. If the awaited
  turn already closed it, omit the edge and complete successfully at the clean
  boundary. Never replay a close edge during repair.
- `assign_window` is local, bounded, idempotent, and fallible only with a fatal
  backend-contract error.
- Initial replay is the sole assignment exception: accumulate identities and
  focus references without allocating or assigning, then preflight and
  reconcile the whole batch at the final barrier before assigning each live
  identity exactly once.
- One repair allowance belongs to the entire transaction chain.
- Reserve and exhaustion-check the repair response ticket before requesting its
  turn; exhaustion issues no request.
- Ticket zero is invalid; allocation never wraps or reuses an id.
- Ticket exhaustion is checked at every origin: external desired/close,
  replay, spontaneous/key, draining follow-up, repair, and shutdown discard.
  No exhausted path requests or answers a turn.
- One `service` call is one nonblocking backend quantum and returns at most one
  public event. Real hard-bound evidence belongs to #40 under ADR 0021.
- The library enforces the Accepted 256-window, 64-binding, and
  256-live-event-per-turn envelope before partial reduction or response. Initial
  replay alone permits 259 events: 256 opens, optional ordinary focus, optional
  exclusive focus, and the final barrier. It is projection-free; a later
  selected-output workarea response gates the first projection and Live.
- The production #40 handoff independently caps backend-local live objects at
  16 outputs, 16 seats, 64 input devices, and 64 libinput devices including
  pre-done objects. Each kind has a checked nonzero never-reused u64 creation
  ordinal. Overflow installs no map/child state and destroys owned proxies once;
  removal frees only the live slot. Unused device strings/arrays are discarded
  in their bounded dispatch quantum.
- The 256-effect cap applies across the entire active transaction and every
  tagged follow-up. Close edges are independently bounded by the window limit,
  retain first-occurrence report order, and are unique per still-live target.
- During shutdown, every offered turn receives a fresh neutral discard response
  without Realm reduction; the deadline may abandon pending discard sequencing.
- `SessionUpdate`'s closed shape orders persistence, state, completion, then
  effects and carries `pending_action`, final `action_completion`, and nonfatal
  diagnostics explicitly. A local ticketless completion returns its immediate
  action result with neither pending nor final ticket field populated. Quit
  uses a typed NoRequester/OriginalAction/CurrentControlRequest barrier.
- A typed action result is closed: application-class `Err` is an immediate
  ordinary error, fatal-class `Err` terminates without an ordinary response,
  ticketless `Ok` is synchronously final, and only `Ok` with
  `pending_action: Some(ticket)` is admission rather than completion.
- Do not install local packaging tools; package builds remain CI-only.

## Task 0 — Make one response drain observable

**Files:** `crates/realm-control/src/server.rs`, `protocol.rs`, exports, and
tests.

### RED

- `response_receipt_settlement_is_exact_and_nonblocking`
- `response_receipt_never_reopens_after_later_request`
- `response_barrier_masks_reads_and_prioritizes_exact_writer`
- `response_barrier_hup_only_settles_closed_without_expiry`
- `terminal_readiness_table_is_closed_for_every_transport_state`
- `response_sequence_exhaustion_is_typed_peer_local_and_never_wraps`
- `oversized_completion_returns_no_receipt_and_proves_closed`

Add and run these seven tests against the absent API first; their compile failure
is the required RED. Do not change `complete_request`, add
`response_settlement`/`begin_response_barrier`, add `ReadyEvent::terminal`, or
add receipt/barrier state before recording that failure.

### GREEN

Allocate a strictly increasing nonzero response sequence per connection.
Pending names queued/partial output, Drained names that exact generation after
its final byte, and Closed covers every record removal. Retain enough bounded
per-live-connection settled state that a later request cannot reopen an older
receipt. Sequence exhaustion returns the typed peer-local error and oversized
completion returns no receipt; both prove that only that peer closed and never
wrap. Implement `ReadyEvent::terminal`, fatal `ListenerTerminal`, the complete
listener/peer/subscriber/output terminal-readiness table,
`ResponseBarrierConflict`, and the total
response barrier: Pending masks listener/all peer reads and exposes only that
receipt's write/terminal progress; HUP-only settles Closed; simultaneous input
and writable ignores input and writes; the same receipt is idempotent; a
distinct Pending receipt is rejected without replacement; and Drained/Closed
installs nothing. Run the seven tests and `cargo test -p realm-control --locked`.

## Task 1 — Replace the backend seam

**Files:** `crates/realm-session/src/backend.rs`, fake implementations in
`session.rs`, `turn.rs`, and `persistence.rs`.

### RED

Change `backend::tests::trait_exposes_the_accepted_backend_contract` first so a
fake must expose:

- `BackendTicket`, `BackendPolicyTurnId`, `BackendBindingId`;
- binding mechanism/event/turn/response types, tri-state next-key edge, and the
  Accepted capacity constants;
- `BackendSubmission`, poll interest, and readiness;
- borrowed `event_fd(&self) -> BorrowedFd<'_>` plus
  `BackendExitPolicy { enabled, watched_modifiers }`;
- local `assign_window -> Result<(), BackendContractError>`;
- `configure_bindings`, `request_policy_turn`, `respond_policy_turn`;
- bounded `service` and
  `begin_exit_session(&mut self, BackendExitPolicy) -> BackendResult<()>`;
- `PolicyTurn`, `OperationCompleted`, empty drain marker, and `Disconnected`.

Before implementation, also add and run
`backend::tests::backend_window_id_accepts_exact_bounds_and_rejects_invalid_bytes`.
Its compile-failing RED requires the private checked 1..=32 printable-ASCII
constructor; no fake or Session preflight may stand in for that production type.

The production policy-turn-id allocator belongs to RiverBackend in #40, so this
slice does not add or test a fake-owned watermark. #40 must add
`backend::tests::policy_turn_id_exhaustion_is_fatal_without_exposure` against
that production allocator and prove typed `Capacity { PolicyTurnIds, .. }`
before exposure, without zero, wrap, or reuse. In this slice run the seam
contract test and record failure for the absent contract.

### GREEN

Add only the seam and mechanical fake adapters. Use private fields plus public
checked constructors/`TryFrom` and getters for nonzero ids so out-of-crate
backends can allocate turn ids. Add closed typed protocol/capacity errors and a
checked `BackendWindowId` constructor/accessor so invalid snapshot identities
are unrepresentable, plus a closed typed protocol/capacity error and a
typed Session event path for fatal assignment-contract failures. Add the new seam
alongside deprecated legacy methods temporarily so the whole Rust crate remains
compilable. The compatibility seam remains through Tasks 1-3; only Task 4
migrates every remaining production/test call site and then removes `apply`,
`focus`, `close`, and `next_event`. Fakes may answer synchronously with
`Complete` until behavioral tests demand `Pending`.

Run the contract test after migrating the mechanical fake adapters needed for
that target to compile, then run the existing crate suite against the temporary
compatibility seam. Do not claim the Accepted interface is complete while the
legacy methods remain. Tasks 2-3 use targeted new tests and may not claim the
whole crate green while old semantic tests still exercise compatibility paths.
Commit this partial state locally as the Task 1 review/recovery checkpoint after
its targeted and existing crate tests pass. Do not push, merge, or present that
commit as a delivered interface while `Pending` remains for Tasks 2–3.

## Task 2 — Linearize external work and policy batches

**Files:** `crates/realm-session/src/session.rs`,
`crates/realm-core/src/keys.rs`, and their tests.

Add transaction substates equivalent to `Idle`, `AwaitingExternalTurn`,
`AwaitingInternalTurn`, `InFlight`, `AwaitingDrain`, `RetryReady`, and
`AwaitingRepairTurn`.
The active record retains committed authority C, visible state V,
last-committed-clean projection L, private working state W, most recently
completed private projection Q, optional original action result, one repair bit,
the response contents, and staged effects.

### RED: turn and admission

Write and individually run these failing tests before implementation:

- `binding_configuration_contains_mechanism_not_policy`
- `unknown_binding_id_is_fatal_before_partial_reduction`
- `policy_batch_is_atomic_and_linear`
- `policy_turn_must_be_answered_exactly_once`
- `resize_binding_is_answered_in_the_same_policy_turn`
- `eaten_unbound_key_returns_to_nav_in_its_policy_response`
- `non_action_policy_turns_are_answered_exactly_once`
- `clean_projection_equality_commits_without_consuming_a_backend_ticket`
- `ticketless_local_update_is_final_without_action_completion`
- `direct_spawn_is_ticketless_effect_only_and_rejects_invalid_command`
- `direct_spawn_is_not_ready_during_active_transaction`
- `visible_metadata_is_json_bounded_at_ingress`
- `repeated_title_changes_replace_bounded_metadata`
- `maximum_show_ledger_fits_one_control_frame`
- `desired_action_requests_turn_and_returns_pending_origin`
- `failed_turn_request_is_error_atomic`
- `failed_turn_request_consumes_origin_ticket`
- `close_target_closed_in_awaited_turn_is_not_replayed`
- `failed_close_turn_request_is_an_immediate_clean_error`
- `close_request_is_a_no_op_without_a_focused_window`
- `duplicate_close_edges_are_unique_and_bounded`
- `identity_binding_is_local_idempotent_and_contract_closed`
- `open_then_focus_and_title_in_one_turn_binds_once_in_order`
- `exclusive_focus_true_then_false_clears_and_restores_ledger_focus`
- `ordinary_focus_observation_cannot_change_ledger_policy`
- `policy_event_limit_accepts_exact_max_and_rejects_max_plus_one_atomically`
- `combined_effect_limit_rejects_external_plus_batch_overflow_atomically`
- `managed_window_limit_accepts_256_and_rejects_257_before_assignment`
- `binding_limit_accepts_64_and_rejects_65_before_configuration`
- `replay_barrier_is_single_final_and_forbidden_elsewhere`
- `initial_replay_is_silent_and_rebinds_each_identity_once`
- `initial_replay_is_projection_free_and_stays_unpublished`
- `replay_barrier_reconciles_without_projection`
- `initial_replay_rejects_workarea_before_assignment`
- `first_selected_workarea_projects_and_publishes_once`
- `initial_workarea_projection_with_exclusive_focus_suppresses_window_focus`
- `initial_workarea_enables_nav_bindings_after_disabled_replay`
- `pre_workarea_authoritative_turns_accumulate_until_revision_one`
- `pre_workarea_binding_input_is_fatal_before_reduction`
- `pre_barrier_non_replay_events_are_protocol_errors`
- `maximum_replay_with_focus_and_barrier_is_accepted`
- `maximum_policy_batch_preserves_input_order_and_is_bounded`
- `policy_text_and_modifier_bounds_are_atomic`
- `exhausted_watermark_never_allocates_the_sentinel`
- `armed_repeat_requests_internal_turn_without_action_completion`
- `released_repeat_is_noop_before_request`
- `new_repeatable_press_replaces_without_resuming_older_target`
- `mode_change_disables_armed_repeat_target`
- `repeat_tick_during_internal_in_flight_is_consumed_without_second_request`
- `repeat_timer_directives_cover_final_press_pending_release_mode_and_quit`
- `keys::tests::only_directional_focus_and_swap_bindings_repeat`

Each scripted fake records requests, turn ids, response tickets, full responses,
and calls after fatal state. Tests inspect Session decisions, not values echoed by
the fake. The repeat-origin cases are intentional internal library tests:
Accepted SPEC 0003 §3/A11b requires a timer fire to recheck the sole exact
successful held, configured, repeatable, committed-enabled target, to request
an internal-origin turn without an ActionCompletion when needed, and to become
a no-op after release/stop, a disabling mode transition, or while a transaction
is already active. A newer finalized target replaces the old one permanently.

### GREEN: admission and response

- Configure mechanism ids once and keep policy mapping in Session.
- For response-requiring external work, reserve an origin ticket, stage the
  candidate/close target, call `request_policy_turn`, and return only
  `pending_action` on success. On error-atomic initial `Io`/`Unsupported`,
  discard staged state, return Error immediately, and retain the consumed ticket
  watermark.
- Accept state-exact turns: Idle and Awaiting states accept untagged turns;
  InFlight accepts none; AwaitingDrain accepts only its tagged turn or marker;
  RetryReady accepts no event.
- Fold external candidate first, then turn events in report order. Bind retained
  opens locally, resolve every compositor-origin reference from BackendWindowId
  only after its current-turn/current-incarnation open, and bind before a
  placement names them.
- During initial replay, keep authoritative workarea absent, allocate and assign
  nothing, and accumulate opens plus permitted focus facts. Reject workarea in
  that first turn: River cannot create the dependent layer-shell output object
  until the replay reports its output. At the barrier, preflight all
  ids/capacities and reconciliation first, then allocate and assign each
  surviving identity exactly once. Answer without projection or closes, keep
  bindings disabled with next-key Preserve, and remain FinalizingReplay after
  its clean boundary.
  Accept later ordinary authoritative facts without publication. The first
  selected-output workarea stages the forced complete projection and desired
  bindings; only its clean final boundary enters Live and publishes revision 1.
  A pre-workarea window/title/focus/exclusive turn is answered projection-free
  and appears only in that revision 1. Any binding press/release/repeat-stop or
  unbound-key input before Live is fatal before partial reduction or response.
- Implement `request_spawn(Vec<String>)` in this task: only Live+Idle accepts
  it, an empty vector or empty program returns `InvalidSpawnCommand`, and a
  successful request is synchronously final, consumes no backend ticket or
  turn, and returns exactly one Spawn effect.
- Normalize observed app ids/titles at ingress to the Accepted 40/80-byte
  JSON-content caps, counting escape expansion and truncating at a Unicode
  scalar boundary with a final U+2026. Replace bounded metadata on coalesced
  updates; never retain the unbounded source string. Prove the 256-window
  all-orbit response bounded-encodes within one control frame.
- Reduce focus inside the same turn. Exclusive true suppresses focused
  placement/border and clears finalized visible title; exclusive false restores
  ledger focus. Ordinary effective-focus observations only acknowledge or make
  the response reassert ledger policy and never mutate the ledger.
- Derive one complete response. Apply close-race omission and one-shot edge
  rules, deduplicate repeated closes in first-occurrence report order, and
  enforce the 256-window close-list bound. Enforce the 65,535-byte total turn
  text cap and canonical sorted unique modifier subsets of at most four in
  Session before partial reduction, independently of the backend. Reject every
  envelope violation atomically. Delay state/effect publication and repeat
  arming until finalization; a failed press never arms from retained held state.
- `fire_key_repeat()` rechecks the sole exactly armed, still-held, configured,
  repeatable, committed-enabled binding. A finalized repeatable press replaces
  and restarts it; only release/stop of the current id disarms it, and an older
  held target never resumes. A finalized disabling mode transition also
  disarms it. While any transaction is active, consume the tick unchanged,
  retain the target, and allocate/request nothing. Otherwise complete locally
  or reserve a fresh internal response ticket and request a turn without
  exposing `pending_action` or `action_completion`. Mark only directional Focus
  and Swap bindings repeatable in the MVP keymap. Add the immediate
  `RepeatTimerDirective` field to every `SessionUpdate`; Preserve is default,
  each successfully finalized repeatable press emits Arm with the Accepted
  600 ms delay/40 ms interval even for the same target, and current
  release/stop, finalized disabling mode, QuitPending, and ShuttingDown emit
  Disarm before persistence/results/effects.
- On `Complete`, finalize immediately. On `Pending`, retain the entire private
  transaction and expose no result.
- For clean local actions, return the completed result immediately in the
  method's `SessionActionResult<SessionUpdate>` while leaving `pending_action`
  and `action_completion` empty; ticket-bearing `action_completion` remains
  exclusive to an origin transaction's final boundary.
Run `cargo test -p realm-session --locked
ticketless_local_update_is_final_without_action_completion` first, then repeat
that targeted command separately with each other literal test name above. Do
not use the realm-session package command for the realm-core keymap test; run
`cargo test -p realm-core --locked only_directional_focus_and_swap_bindings_repeat`
for that RED/GREEN cycle. Do
not run or claim the whole crate yet: legacy production and fixture paths
intentionally remain until Task 4.

## Task 3 — Completion, drain, repair, and replay

**Files:** `crates/realm-session/src/session.rs` and its tests.

### RED: success boundaries

- `desired_projection_commits_only_after_matching_success_and_drain`
- `projection_changing_drain_completes_original_action_only_after_followup`
- `complete_followup_finalizes_original_action_without_waiting_for_marker`
- `empty_observation_drain_clears_the_gate`
- `retained_policy_turn_replaces_empty_drain_marker`
- `mismatched_completion_cannot_commit_the_active_candidate`
- `completion_without_an_active_operation_is_rejected`
- `unexpected_observation_drain_marker_is_rejected`
- `complete_bootstrap_response_remains_finalizing`
- `pending_bootstrap_drain_workarea_projects_before_live`
- `initial_workarea_projection_enters_live_without_marker`
- `initial_workarea_projection_waits_for_success`
- `observed_only_terminal_success_promotes_snapshot_before_drain`
- `retained_window_open_is_bound_before_followup_projection`
- `close_request_waits_for_the_observed_close_before_mutating_state`
- `persistence_exposes_only_authoritative_live_state`

For desired/close/key-mixed/replay work, prove C/V/L and persistence remain
unchanged through pending success. Separately prove observed-only terminal
success promotes its facts into C and `snapshot()` before drain while V/L stay
old. Prove finalization order is persistence observation, visible state, then
the original action completion.

The bootstrap-drain test must make the projection-free replay response return
Pending, then deliver the first selected-output `WorkareaChanged` inside its
correctly tagged retained turn. That tagged response carries the first forced
projection and Nav binding set; Session stays unpublished until the tagged
response reaches its own clean boundary, then enters Live exactly once.

### RED: failure and repair

- `desired_projection_failure_drains_then_repairs_authoritative_state`
- `late_followup_failure_rolls_back_entire_external_transaction`
- `failed_action_result_waits_for_clean_repair_boundary`
- `complete_repair_returns_retained_error_at_that_boundary`
- `mixed_response_failure_retains_facts_and_repairs_state_without_replaying_edges`
- `failed_mode_response_rolls_back_mode_and_suppresses_effects`
- `failed_chord_ensure_is_cancelled_not_replayed`
- `failed_unbound_key_nav_response_rearms_resize_once`
- `key_policy_only_unsupported_is_diagnostic_then_repairs`
- `key_origin_desired_failure_reports_diagnostic_and_repairs`
- `key_origin_close_quit_failure_reports_diagnostic_without_completion`
- `repeat_internal_failure_reports_diagnostic_and_repairs`
- `observed_only_io_is_nonfatal_and_waits_for_drain`
- `observed_projection_failure_retains_fact_until_repair_succeeds`
- `observed_only_terminal_failure_exposes_authoritative_snapshot_before_repair`
- `workarea_authority_does_not_expand_snapshot_v1`
- `show_ledger_waits_with_get_state_during_observed_repair`
- `observed_only_clean_component_failure_drains_without_repair`
- `focus_mismatch_equal_projection_failure_requires_repair`
- `failed_apply_marks_projection_dirty_until_a_complete_repair`
- `unsupported_apply_rolls_back_and_emits_no_state`
- `bootstrap_pending_failure_classes_are_phase_correct`
- `pending_initial_workarea_projection_repairs_before_entering_live`
- `failed_pending_close_does_not_dirty_projection`
- `backend_loss_during_close_is_fatal`
- `protocol_and_capacity_results_are_always_fatal`
- `second_projection_failure_is_fatal`
- `fatal_backend_errors_never_schedule_repair`
- `unsupported_authoritative_projection_is_fatal`
- `disconnect_during_observation_drain_is_fatal`
- `close_after_drain_marker_cannot_poison_repair`
- `repair_ticket_exhaustion_does_not_request_turn`
- `backend_ticket_exhaustion_never_wraps_or_submits`
- `backend_work_gets_one_retry_and_gates_event_reads`
- `pending_backend_work_gates_actions_and_publication`
- `active_transaction_defers_module_and_whichkey_publication`
- `shutdown_policy_turn_is_answered_without_reduction`
- `shutdown_answers_offered_turn_despite_abandoned_drain_tag`
- `quiescing_completion_result_classes_are_total`
- `exiting_rejects_non_disconnect_and_accepts_expected_disconnect`
- `begin_shutdown_abandons_every_state_and_is_idempotent`
- `begin_exit_session_is_exactly_once_and_rejects_wrong_phase`
- `begin_exit_session_backend_failure_is_fatal_without_phase_change_or_retry`
- `direct_quit_transmutes_each_active_transaction_state_without_new_ticket`
- `direct_quit_is_idempotent_in_quit_pending`
- `direct_quit_rejects_every_nonlive_phase_without_mutation`
- `quit_pending_policy_turn_is_answered_without_reduction`
- `mixed_action_and_quit_enters_quit_pending_after_completion`
- `spawn_then_quit_preserves_pre_quit_effect_order`
- `quit_then_spawn_suppresses_post_quit_effect`
- `quit_barrier_suppresses_effects_in_later_draining_turn`
- `staged_effect_cap_applies_across_drain_chain`
- `failed_mixed_release_keeps_repeat_disarmed`
- `failed_mixed_repeat_stop_keeps_repeat_disarmed`

The bootstrap failure table must exercise a Pending projection-free response,
not the later projection-bearing workarea response. `Io` drains and spends the
single neutral repair to reassert the backend-private selected default plus the
disabled/Preserve binding state, then remains FinalizingReplay without
publication. `Unsupported` is fatal without repair, publication, or Live.

Also prove a dirty empty marker enters RetryReady; the next driver turn first
reserves the repair ticket, then requests a policy turn without service; the
next untagged turn receives that ticket. Exhaustion makes no request. Request
`Io`/`Unsupported`, response error, or later terminal failure exhausts the one
repair. Loss remains fatal.

Exercise `backend_ticket_exhaustion_never_wraps_or_submits` as one table-driven
matrix covering external desired and close origins, initial replay,
spontaneous/key and internal-repeat turns, tagged draining follow-ups, repair,
and shutdown discard. Also retain the separate
consumed-origin-after-request-failure assertion. Every row proves zero is
absent, `u64::MAX` is not reused, and no request/response is issued.

The fresh-Ensure test must distinguish the consumed committed edge after
`UnboundKeyEaten` from the failed response's uncertain edge: restore Resize
with one new Ensure, while an uncertain Ensure receives Cancel and is never
replayed.

Use a state-table fixture for direct Quit across `Idle`,
`AwaitingExternalTurn`, `AwaitingInternalTurn`, `InFlight`, `AwaitingDrain`,
`RetryReady`, and `AwaitingRepairTurn`. The first call abandons semantic
W/origin/effects without completion or a new ticket and returns the
CurrentControlRequest barrier; a second call in `QuitPending` is unchanged.
Preserve already-requested external/internal/repair tickets only for neutral
discard sequencing and drop `RetryReady`, where no request exists.
Add a lifecycle-phase table proving direct Quit returns the typed invalid-phase
error without mutation in InitialReplay, FinalizingReplay, ShuttingDown,
Exiting, and ExitComplete; only Live and QuitPending are successful.

Use a second table for `QuitPending`/`ShuttingDown` discard sequencing: idle
untagged turn, retained outstanding request, matching completion, tagged drain,
empty marker, later offered turn, abandoned-work late completion, offered turn
with an arbitrary old/future drain tag, deadline abandonment, ticket
exhaustion, and response failure. In `ShuttingDown`, late completions are
ignored and every offered turn is answered regardless of tag. Every response has no
projection/closes, committed enabled/watch sets, and Cancel; no Realm fact,
publication, persistence, action completion, or ordinary effect escapes.
The begin-exit tests also assert Session constructs exactly one
`BackendExitPolicy` from its last committed enabled/watch sets; projection,
closes, and non-Cancel next-key state are unrepresentable in that type.

### GREEN

Implement exact-ticket terminal handling before ordinary phase dispatch. Store
one immutable transaction-wide semantic failure base at admission. Any
repairable failure before the final clean boundary rolls back every reversible
desired/key/effect delta since that base, including deltas carried through an
earlier successful response, while retaining every compositor/input-safety fact
across the chain. Classify mixed
responses component-wise: fatal > repair > clean close-only. Conservatively
dirty every attempted uncertain component: projection whenever the recorded
response contained `Some`, and bindings when enabled/watch values differed,
their cache was already dirty, or an edge was emitted. Never replay close edges
or an uncertain next-key Ensure, and emit Cancel for that uncertainty.

Branch authority timing explicitly. When an observed-only response safely
finishes, install every compositor fact into in-memory C and expose the closed
V1 `snapshot()` at once, including on repairable terminal `Io`; include only
persistable close/mapping/watermark/orbit consequences, never workarea/title or
other metadata absent from V1. Keep V/L old until the final clean boundary.
Facts in a tagged turn wait for that turn's own response finish.
Maintain an immutable visible ledger-plus-metadata snapshot at the same final
boundary as `Session::state()` and expose it only through
`visible_ledger(Option<OrbitId>) -> Vec<OrbitLedger>` for ShowLedger. Keep the
existing authoritative accessors out of the control adapter.
Desired, close, key/effect-mixed, and replay W remains private until finalization.
If the response attempted no stateful projection/binding component, an
observed-only failure drains without requesting artificial repair.

Treat startup finalization as two explicit cases. The projection-free replay
response commits reconciliation and identity bindings but keeps V absent,
keeps protocol bindings disabled, and remains FinalizingReplay after either a
synchronous or drained clean boundary. The first later selected-output
workarea response uses the ordinary pending/drain/repair machinery for a forced
complete projection and desired bindings; only its clean final boundary derives
revision 1 and enters Live. Intermediate authoritative turns are answered and
accumulated without publication, and binding input before Live is a protocol
error.

Finalization performs, as one Session transition:

1. emit the immediate repeat-timer directive implied by retained input/final policy;
2. install W into committed authority;
3. install final clean projection/binding state and clear active/dirty/retry;
4. expose the persistence snapshot;
5. derive/install V and revision;
6. return visible state, then the original action result, then ordinary staged
   effects.

A staged Quit returns a `QuitPending` effect after the original action result
and stops further admission. The #38 adapter must not call `begin_shutdown`
until that request frame is fully written or its connection closes. A key-only
Quit with no requester begins after final-boundary persistence/publication.
Test the library ordering here; retain the real socket-drain assertion for #38.

Derived process effects retain report order through all tagged follow-ups, and
the 256-effect budget belongs to the whole active chain rather than resetting
per turn. The first Quit is sticky and terminal for later derived
actions/effects, though later authoritative facts still reduce.

In `QuitPending` and `ShuttingDown`, do not reduce policy events. QuitPending
preserves the already reserved response ticket for an
external/internal/repair request that has not yet received its untagged turn,
and preserves an in-flight ticket or awaited drain until its matching protocol
boundary. ShuttingDown forgets abandoned ticket identity and semantic content,
but inspects result class. QuitPending matching and ShuttingDown late
`Ok`/`Io`/`Unsupported` completions advance/disappear only as discard protocol;
`Protocol`/`Capacity` and pre-exit `Disconnected`/`Unavailable` remain fatal.
`Session::begin_shutdown` is a total idempotent transition into ShuttingDown.
On first call it abandons all semantic/protocol candidates, origins, tickets,
closes, effects, and retry-ready repair while preserving the consumed-ticket
watermark, calls no backend method, and emits only repeat-timer Disarm. Repeated
calls emit unchanged. ShuttingDown answers every offered turn with a fresh
no-projection/no-close discard response regardless of its drain tag, carrying
committed binding state and next-key Cancel. Track a Pending discard only for
orderly progress; the response barrier or fixed deadline may abandon it.
Response error or ticket exhaustion is fatal. `Session::begin_exit_session` is
legal exactly once in ShuttingDown; it calls the backend exactly once and enters
Exiting only on success. Wrong-phase/repeated calls are contract errors. #38,
not Session, proves the external control-drain, worker-fence, and exposed-turn
preconditions before calling. A backend error is fatal,
does not enter Exiting, and is never silently retried. Session issues no later
public request/response and rejects any public event other than the expected
post-flush Disconnected. The backend's call is the local exit cutoff: it cancels
any prepared read, stops ingress admission, preserves already emitted/queued
bytes and finishes any manage/render phase already open or parsed from the
accepted immutable response without replacement, and suppresses its public
completion/drain. A Pending response awaiting a kernel-unread future
`render_start` abandons that later render phase. The backend uses the fixed exit
policy only for an admitted/open/queued sequence not yet exposed and answered,
flushes every applicable open-phase finish before queuing and flushing the exit
request, and
terminally supersedes kernel-unread bytes and incomplete decoder fragments;
real River evidence remains #40-owned.

Run `cargo test -p realm-session --locked
desired_projection_commits_only_after_matching_success_and_drain` first, then
repeat that targeted command separately with each other literal test name
above. Do not run or claim the whole crate yet; the legacy compatibility seam
and its unmigrated callers remain until Task 4.

## Task 4 — Complete migration and one backend service decision

**Files:** `crates/realm-session/src/backend.rs`,
`crates/realm-session/src/turn.rs`,
`crates/realm-session/src/persistence.rs`,
`crates/realm-session/src/session.rs`, and all realm-session tests/fakes.

### RED

- `backend_turn_drives_active_transaction_but_requests_retry_before_service`
- `backend_turn_services_immediate_and_writable_work_once`
- `immediate_none_progress_is_rechecked_before_blocking`
- `service_error_abandons_pending_operation_fatally`
- `session_update_api_has_only_the_accepted_closed_shape`
- `session_exposes_read_only_backend_poll_registration_seam`
- `legacy_backend_methods_are_absent_after_migration`
- `expected_exit_disconnect_returns_exit_complete_once`

Prove RetryReady requests its turn with zero service calls. AwaitingExternalTurn,
AwaitingInternalTurn, AwaitingRepairTurn, InFlight, and AwaitingDrain continue service. No interest or
readiness is idle; otherwise exactly one service call occurs. `None` is
Progressed and requires an interest recheck. A backend with no prepared guard
advertises immediate preparation work; its one `None` preparation quantum must
run before the helper can return the subsequently idle state to an outer poll.
Handling one nonfatal event that leaves RetryReady returns
`RetryScheduled(SessionUpdate)`; it preserves the update, carries no fabricated
fatal error, and forces the next helper invocation before poll. The API-shape
test proves the retired `BackendWorkPending` error variant is absent.
The expected Exiting disconnect returns `BackendTurn::ExitComplete` once; the
helper rejects a later invocation because the outer driver must stop the target.

### GREEN

Implement public read-only `Session::backend_event_fd()` and
`Session::backend_poll_interest()` delegation for #38, plus the pure service
decision, then migrate every caller to
`backend_turn(&mut Session<_>, BackendReady, Instant)`. The helper
queries `backend_poll_interest()` itself; an all-false pre-poll readiness still
services RetryReady/immediate work, while post-poll invocations receive exactly
one fresh readiness snapshot. Mechanically migrate every remaining production
caller plus the `turn.rs`, `persistence.rs`, and `session.rs`
test/fake implementations from `apply`/`focus`/`close`/`next_event`/`workarea`
to policy turns and bounded service. Explicitly migrate every `SessionUpdate` producer and
consumer to the accepted fields `repeat_timer`, `persistence`, `state`, `pending_action`,
`action_completion`, `effects`, and `diagnostic`; remove the old
`projection_applied`, deferred-event, and close `Option<WinId>` surfaces.
Finally remove the temporary legacy methods and out-of-band `workarea` accessor
from `WmBackend` and make the
contract/compile test prove that only the Accepted seam remains.

Do not implement the outer poll loop or claim ADR 0021's real Wayland bound
from a fake. In particular, prepared-read ownership and actual poll revents are
deferred to their exact #38/#40 owners below.

Run all eight named Task 4 RED tests individually before implementation and
again after GREEN. Then run
`cargo test -p realm-session --locked`, `cargo test --workspace --locked`,
format, and clippy. This is the first task allowed to claim the whole crate and
workspace green.

## Task 5 — Verify and hand off remaining MVP work

Run repository documentation and namespace guards and
`git diff --check origin/main...HEAD`; preserve Task 4's recorded full
crate/workspace, format, and clippy evidence.

Commit the verified slice. Update issues #38, #40, #61, #62, #63, and #65 with the
repository's file-backed GitHub body helper so live Symphony work cannot retain
superseded APIs or criteria. For #38 leave these open:

- #38 A1/A2 startup construction and failure evidence:
  `realm_session::tests::startup_seeds_six_orbits_with_first_active` and
  `realm_session::tests::backend_startup_failure_prevents_listener_and_readiness`;
- #38 A14i production entrypoint evidence:
  `realm_session::tests::production_daemon_wires_every_runtime_owner`, followed
  by #40's real headless-River
  `backend::tests::headless_river_daemon_reaches_ready_serves_get_state_and_quits`.
  That production smoke uses a real `realm_control::Client` Hello/GetState/Quit
  flow and adds no CLI surface.
  #74/#75/#76 must launch the installed `realm-wm` in their packaged-session
  lanes; #74 removes the `pending_m2` guard and enables the real
  `checks.x86_64-linux.session-boots` body. Library-only/fake-backend driver
  tests cannot close this obligation;

- #38 A28 total Ready-request dispatch:
  `realm_session::tests::request_dispatch_table_is_total`,
  `realm_session::tests::spawn_response_reserves_worker_before_acknowledgement`,
  and `realm_session::tests::invalid_orbit_and_retired_reload_are_application_errors`,
  proving every Request variant maps to the Accepted typed owner, direct Spawn
  reserves one worker slot before Ok and acknowledges admission rather than
  process success, ShowLedger validates one-based input and uses the visible
  boundary, and ReloadTheme calls neither Session nor a worker;

- #38 A28 `ConnectionId` routing with
  `realm_session::tests::mixed_action_and_quit_drains_response_before_shutdown`
  and
  `realm_session::tests::subscriber_publish_failure_does_not_suppress_requester_completion_or_quit`,
  proving persistence → publication → request completion → pre-Quit effects
  and the exact response-receipt barrier. The mixed-action test must assert that
  `QuitAfter::OriginalAction`, like `CurrentControlRequest`, installs the barrier
  and masks ordinary interests;
- #38 `realm_session::tests::requester_close_outcomes_commit_effects_and_begin_shutdown_once`,
  table-driven adapter evidence that `StaleConnection`,
  `ResponseSequenceExhausted`, and `OutboundFrameTooLarge` completion outcomes
  all prove Closed, call `begin_shutdown` exactly once, and commit reserved
  pre-Quit effects for both requester-bearing Quit paths;
- #38 A14f/A31
  `realm_session::tests::snapshot_classification_precedes_window_manager_binding`,
  `realm_session::tests::pre_listener_recovery_pump_reaches_live_before_activation`
  and `realm_session::tests::readiness_follows_live_listener_activation`,
  including delayed snapshot-worker completion before River binding and
  delayed replay/finalization/repair
  before listener activation and readiness;
- #38 A12/A32/A41 combined-loop and poll-set evidence: backend-first and
  zero-poll fairness across backend, repeat timer, clock timer, worker eventfd,
  and control sources; `combined_loop_orders_all_ready_sources_once`,
  `received_release_crosses_ingress_dispatch_dequeue_before_repeat`,
  `continuous_backend_readability_yields_after_one_admitted_epoch`, and
  `repeat_overrun_during_pending_work_is_dropped_without_catchup`; conditional
  `POLLIN`, `POLLOUT`, independent `POLLERR | POLLHUP` mapping into
  `BackendReady::readable` and `BackendReady::terminal`, HUP-only guard
  consumption through `backend_hup_only_maps_to_terminal_readiness`,
  `backend_pollnval_is_fatal_without_service`, and
  `idle_backend_prepares_read_before_poll`. The #38 idle test
  proves the outer loop invokes the idle prepare-read handoff before blocking;
  it does not claim the real guard primitive;
- #38 `realm_session::tests::subscriber_deadline_expires_without_new_state`,
  `realm_session::tests::subscribe_initial_state_uses_last_visible_session_boundary`,
  and `realm_session::tests::control_quantum_rechecks_backend_before_next_peer`
  for A13/A14d/A14e control deadline, visible-ledger/state adapter, bounded
  control quantum, and readiness-recheck integration;
- #38 `realm_session::tests::quit_pending_masks_control_reads_but_drains_exact_response`,
  including simultaneous readable/writable and HUP-only barrier readiness;
  a supposedly unreachable `ResponseBarrierConflict` is fatal and leaves the
  first barrier unchanged;
- #38 `realm_session::tests::worker_process_queue_accepts_256_and_rejects_257_atomically`,
  `realm_session::tests::worker_result_queue_blocks_worker_not_event_loop`,
  `realm_session::tests::snapshot_slot_is_separate_replaceable_and_accepted_beside_256_process_jobs`,
  and `realm_session::tests::worker_seal_rejects_new_jobs_without_consuming_queue_capacity`
  for exact/max-plus-one worker capacity, separate snapshot coalescing,
  noncoalescible process-effect reservation, and no partial jobs;
- #40 `backend::tests::synchronous_flush_with_admitted_postturn_work_returns_pending`,
  proving a synchronous finish cannot return Complete while a decoder fragment
  or queued protocol message admitted before the decision could still yield a
  retained fact; matching Ok then precedes exactly one tagged turn or marker;
- #40 A1-A9/A17 production River evidence:
  `backend::tests::river_binds_all_required_globals_and_reports_full_capabilities`,
  `backend::tests::input_syncs_precede_window_manager_bind_without_open_manage_sequence`,
  `backend::tests::xkb_config_absence_does_not_block_mvp_startup`,
  `backend::tests::missing_or_old_required_global_fails_before_window_management_request`,
  `backend::tests::initial_replay_without_workarea_finishes_then_projects_first_selected_workarea`,
  `backend::tests::first_complete_output_is_stable_default_and_selected_removal_fails_over_by_creation_order`,
  `backend::tests::no_complete_output_survivor_is_typed_unavailable`,
  `backend::tests::first_river_seat_exclusively_owns_realm_input_policy`,
  `backend::tests::missing_or_removed_selected_seat_is_typed_unavailable`,
  `backend::tests::river_object_caps_and_ordinals_are_atomic_for_every_kind`,
  `backend::tests::river_object_removal_frees_slot_without_reusing_ordinal`,
  `backend::tests::river_object_capacity_failure_destroys_proxy_and_children_once`,
  `backend::tests::unused_device_metadata_is_not_retained`,
  `backend::tests::policy_response_orders_manage_then_render_and_finishes_once`,
  `backend::tests::swap_projection_uses_one_render_sequence`,
  `backend::tests::clean_projection_cache_suppresses_identical_river_requests`,
  `backend::tests::focus_change_emits_only_focus_and_border_requests`,
  `backend::tests::stow_hides_inside_render_without_dimension_request`,
  `backend::tests::layer_shell_exclusive_zone_maps_to_workarea_without_close`,
  `backend::tests::configured_bindings_create_one_stable_river_object_each`, and
  `backend::tests::corrective_content_clip_converges_once`;
- #40 A9a/A9b exact input policy evidence:
  `backend::tests::input_defaults_use_existing_default_seat_and_fixed_keyboard_repeat`,
  `backend::tests::removed_input_device_cancels_unfinished_policy_without_reuse`,
  `backend::tests::tap_to_click_is_enabled_only_after_complete_supported_device_snapshot`,
  `backend::tests::tap_request_result_is_total_and_startup_waits_for_success`, and
  `backend::tests::non_tap_input_policy_is_preserved_without_requests`.
  Rewrite #63 around these defaults and remove its false seat-creation and
  configurable-repeat claims. Create or update a separate post-MVP issue for
  runtime keyboard-layout switching; M2 does not bind `river-xkb-config-v1`;
- #40 retained-state and exit evidence:
  `backend::tests::retained_observations_preserve_equivalent_report_order_under_bounds`,
  `backend::tests::retained_observation_capacity_is_fatal_before_partial_exposure`,
  `backend::tests::tombstoned_retained_open_emits_one_close_and_no_placement`,
  `backend::tests::begin_exit_finishes_partially_flushed_open_phase_without_replacement`,
  `backend::tests::begin_exit_abandons_pending_unread_render_phase`,
  `backend::tests::begin_exit_answers_only_unexposed_sequences_with_exit_policy`,
  with the last test injecting an already parsed queued/open `manage_start` and
  separate unread peer bytes and retained received descriptors, proving the
  admitted sequence gets one internal
  response with no projection/closes and Cancel from the supplied
  `BackendExitPolicy`, its finish flushes before the exit request is queued and
  flushed, unread ingress is never admitted after the cutoff, every descriptor
  owned by superseded decoded/partial work closes exactly once, and only then
  may Disconnected succeed;
- #40 `backend::tests::exit_cutoff_disables_read_interest_and_requires_postflush_terminal`,
  proving poll interest disables `POLLIN` while keeping write plus independent
  terminal observation, post-cutoff readable readiness alone performs no
  ingress and cannot return Disconnected, and HUP after full flush does;
- #38 A14g transport shutdown transition and exactly-once
  `realm_session::tests::shutdown_driver_calls_session_exit_once_and_stops_on_exit_complete`,
  including exactly-once `Session::begin_exit_session` and continued
  finite immediate/write/`POLLOUT` service plus independent terminal
  observation until the expected post-flush disconnect yields
  `BackendTurn::ExitComplete` exactly once. The test asserts post-cutoff
  `POLLIN` omission and proves that no public dequeue, dispatch, ingress, read
  preparation, or completion reduction occurs after the cutoff;
- #38 `realm_session::tests::shutdown_fence_orders_full_process_queue_then_latest_snapshot_before_exit`
  and `realm_session::tests::shutdown_fence_timeout_records_degraded_state_then_exits`,
  proving `shutdown_snapshot` is submitted and the worker sealed at the first
  terminal `QuitPending` boundary, before any response-barrier wait or control
  drain, and before the out-of-capacity fence,
  new work is sealed out, max-capacity Spawn then Quit cannot overtake the
  fence, exit waits for acknowledgement, and exact 2,000 ms expiry records a
  degraded diagnostic before exit. First seal fixes `deadline = now + 2,000 ms`,
  repeat seal cannot slide it, and expiry is `now >= deadline`;
- #38 A14h
  `realm_session::tests::direct_quit_waits_for_exact_response_receipt`;
- #38 A29 `realm_session::tests::snapshot_worker_reads_at_most_limit_plus_one`,
  `realm_session::tests::snapshot_worker_rejects_growth_beyond_exact_limit`,
  and `realm_session::tests::snapshot_worker_read_error_prevents_listener_activation`;
- #38 `realm_session::tests::repeat_timer_directives_program_single_timerfd_exactly`
  and `realm_session::tests::clock_and_repeat_overruns_coalesce_without_catchup`
  for A10–A12 armed-only repeat timer and overrun behavior, including every
  Arm/Disarm directive from final press, pending release/stop, disabling mode,
  and Quit;
  the Session-level repeat-origin rules are already covered in Tasks 2–3;
- #65 `realm_session::tests::key_to_manage_finish_meets_four_millisecond_budget_on_reference_linux`
  against the documented reference runner and maximum supported policy turn;
- #40 A3/A5/A9/A12/A17/A20 River object, cache, manage/render/finish and exact
  geometry evidence;
- #40 A40/A43 real policy-turn coalescing, capacity, bounded retention, and
  tombstone evidence, including the production turn-id allocator,
  `river_policy_text_and_modifier_bounds_precede_exposure`, and
  `completion_does_not_emit_empty_marker_before_precompletion_ingress_is_normalized`;
  Session's retained-open assignment/finalization evidence is completed in Task 3;
- #40 A41 every ADR 0021 real-socket guard:
  `protocol_dispatch_is_capped_per_service_quantum`,
  `read_ingress_is_capped_under_continuous_peer_output`,
  `partial_wayland_frame_continues_across_bounded_ingress_quanta`,
  `truncated_ancillary_data_is_fatal`,
  `retained_descriptor_cap_closes_every_owned_fd_on_overflow`,
  `policy_turn_batch_is_capped_without_reordering_inputs`,
  `maximum_policy_turn_meets_the_response_budget`,
  `writable_finish_flush_preempts_continuous_readable_ingress`,
  `prepared_read_is_consumed_or_cancelled_exactly_once`,
  `prepare_read_none_forces_immediate_dispatch_before_poll`, and
  `completion_does_not_emit_empty_marker_before_precompletion_ingress_is_normalized`,
  `flush_would_block_requests_pollout_without_busy_retry`. #40 proves the real
  prepare/read guard and bounded phases; #38 proves outer-loop handoff/revents;
- #65 real Linux key-to-finish timing.

Push, open the PR, run adversarial review, handle incoming PRs, and merge only
after remote CI passes.
