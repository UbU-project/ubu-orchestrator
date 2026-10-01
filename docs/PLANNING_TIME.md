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
then stored Calendar scope, then now plus the configured span.
`UBU_PLANNING_HORIZON_SECONDS` accepts 1 through 2678400 seconds.

**The default span is 604800 seconds, one week.** Until P1B-53 it was 86400, one
day; the operator decided on one week for the switch to mainline planning. One
day is still a supported setting: export `UBU_PLANNING_HORIZON_SECONDS=86400`.

One week of horizon is one week of calendar. The horizon is the range capture
and reconciliation observe, so with nothing set:

- capture takes a week of events, and each instance of a recurring commitment
  inside the week becomes its own occupied-time Task;
- the Plan covers a week: a routine materialises seven times, and a Task that
  fits nowhere today can be placed later in the week;
- **it costs more.** A week-long Plan has several times the Static placements
  of a one-day Plan and takes longer to generate, and a preview after it
  proposes an event for every one of them. The rehearsal in `ubu-devshell`
  measures the generate at both horizons on the same week.

Tests can use `ServerConfig::with_planning_horizon_seconds(seconds)` to state
the horizon they rely on without changing process environment variables, which
every test in a binary shares.
Dynamic placement starts no earlier than now even when a selected scope starts
earlier. **From P1B-53 it starts on a whole minute**: see
[The Plan starts on a whole minute](#the-plan-starts-on-a-whole-minute). Static Tasks in progress stay whole. Repair also floors its adjusted
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

## The Plan starts on a whole minute

Until P1B-53 the planning window was anchored at the exact instant of the
request, so Dynamic packing began at a time like `03:51:12Z`. Two Plans made
seconds apart had different Dynamic windows, the diff between them was
genuine, and every re-plan rewrote every Dynamic event on the calendar. The
diff was correct; the windows really differed.

The anchor is now **rounded up to the next whole minute**. A request at
`08:00:37Z` plans from `08:01:00Z`. A request already on the minute plans from
that minute. It rounds up and never down: rounding down would place work in
the past. The horizon's end is derived from the rounded start, so the span is
still exactly `UBU_PLANNING_HORIZON_SECONDS`.

What that gives:

- **two Plans generated within the same minute have identical Dynamic
  windows**, and the Calendar preview between them proposes **no
  operations**;
- no Dynamic placement is ever earlier than the request.

What it does not give, and should not be read as giving:

- **It removes sub-minute churn, not all churn.** A Plan generated a minute
  later starts a minute later, and a re-plan at noon re-packs the afternoon.
  That is correct: the Plan changed. Keeping a placement where it was across
  re-plans is `UBU-D0279`'s "preserved or frozen placements", a Phase 2
  concept that nothing here implements.
- Only the start of the packing is quantised. Where the planner puts a later
  placement is its own decision, and it can still fall on an odd second.
- Static anchors keep their own windows. The range capture and reconciliation
  observe still begins at the exact instant.
