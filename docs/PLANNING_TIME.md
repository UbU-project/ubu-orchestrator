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

Only horizon construction and next-action selection use the injectable planning
clock. Envelope, log and record timestamps continue to use their existing clocks.
Next action skips ended placements and reports `stale_calendar` when all current
placements have ended. All times are UTC; local day boundaries are deferred.

Known limitations: the unchanged kernel can fail the whole plan when one Dynamic
Task cannot fit in the bounded horizon, including a large backlog against the
24-hour default. Kernel repair also preserves prior Static placements even when
their window changes. No Calendar rows or horizon UI are introduced here.

`ubu-ui` still renders numeric coordinates as minutes in `formatMinuteTimestamp`.
It will display incorrect times until a separate UI change reads `start_at` and
`end_at`; this ticket deliberately leaves that repository unchanged.
