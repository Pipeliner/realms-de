# Task time

Owner-directed repository policy, effective 2026-09-27. The append-only ledger
is `task-time.csv`; the assessment is `mvp-status-2026-09-27.md`.

Record one UTC interval per specific issue/task and actor. Start an interval
when work starts; finish it when switching tasks or handing off. An empty end
means active/unclosed, not proven continuous work. On recovery, flag unknown
gaps rather than charging them to a task. Use a current clock, not estimates.

Categories: implementation, investigation, review, verification, assessment,
or wait. Include task-related spec/test work in that task rather than inventing
separate process tasks. Associate CI waits with a run ID; stop the wait when
switching to other work. CI runtime can be reported separately from GitHub's
timestamps and must not be added to agent active-work time.

Parallel agents each use their own actor and intervals; their summed effort is
not elapsed delivery time. Report measured effort and elapsed time separately.
Do not infer costs or historical time from token usage, commit gaps or task age.
Earlier work is unmeasured. Begin tracking with this assessment; do not backfill.

Use `apply_patch` for ledger updates. No new timer service or tracking platform
is required. At each handoff, include the completed interval and exact next task.
