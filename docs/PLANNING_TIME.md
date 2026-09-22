# Planning time

Store-built requests use Unix seconds for every coordinate and seconds for every
duration. Core duration estimates pass through unchanged. Legacy `duration_minutes`
and `estimate_minutes` multiply by 60; `estimate.seconds` passes through (minimum
one second). Missing durations default to 1800 seconds.

Static windows use `unix_timestamp()` directly, with no minute rounding. Fractional
seconds are discarded because kernel coordinates are integer seconds. Affect
observations, freshness thresholds, deadlines, feedback latency and the 300-second
slack allowance all use the same unit.

Steps persist `start_at` and `end_at` as RFC 3339 UTC strings alongside numeric
`start` and `end`. Caller-supplied full requests keep their unit-agnostic numeric
kernel coordinates unchanged; the timestamp labels interpret those coordinates as
Unix seconds. Coordinates outside the RFC 3339 range return
`invalid_schedule_timestamp` rather than inventing a timestamp.

The horizon applies only to store-built requests: explicit RFC 3339 horizon first,
then stored Calendar scope, then now plus the configured span (default 86400
seconds). `UBU_PLANNING_HORIZON_SECONDS` accepts 1 through 2678400 seconds.
Dynamic placement starts no earlier than now even when a selected scope starts
earlier. Static Tasks in progress stay whole. Repair also floors its adjusted
horizon start at now and retains the existing frozen-step adjustment.

`UBU_PLANNER_STRATEGY` selects `chunked` (default) or `greedy` for both planning
and repair. Any other value is a startup configuration error naming the variable.
Tests can use `ServerConfig::with_planner_strategy(raw)` without changing process
environment variables.

Only horizon construction and next-action selection use the injectable planning
clock. Envelope, log and record timestamps continue to use their existing clocks.
Next action skips ended placements and reports `stale_calendar` when all current
placements have ended. All times are UTC; local day boundaries are deferred.

## Allowed ranges

A Dynamic Task's `allowed_time_range` is intersected with the requested horizon.
The existing absent-Static-prerequisite push applies next, followed by the now
floor. Allowed ranges only narrow the Task window; they never widen the horizon.
An absent Static prerequisite ending at or after the horizon end takes precedence
and produces `dependency_outside_horizon`. Otherwise an empty window or one shorter
than the kernel duration model's `placement_seconds()` produces `task_unplaceable`.
This uses the fixed duration or log-normal mode, not its minimum or p95.

Dynamic dependents of these removed Tasks are excluded to a fixpoint with
`prerequisite_unplaceable`, before any dependency edge is dropped. Kept Static
Tasks break that chain. Preconditions, lifecycle and frozen repair exclusions
retain their existing edge-dropping behavior.

Known limitations: individually oversized Tasks are excluded. The default chunked
sweep rescues priority-first narrow-slot failures, but packing failures that no
fill rule or look-ahead resolves still fail the whole request until partial
placement (`UBU-Q0155`).
Kernel repair also preserves prior Static placements even when their window
changes. No Calendar rows or horizon UI are introduced here.

`ubu-ui` still renders numeric coordinates as minutes in `formatMinuteTimestamp`.
It will display incorrect times until a separate UI change reads `start_at` and
`end_at`; this ticket deliberately leaves that repository unchanged.
