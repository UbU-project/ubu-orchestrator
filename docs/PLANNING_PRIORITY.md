# Task priority

Store-built requests read active, enabled pairwise Task Preferences. Only pairs
whose two Tasks survived all request exclusions count; Objective pairs, disabled
Preferences and pairs involving an ineligible Task do not contribute. Settings
remain named inputs for bootstrap and affect tolerances, separate from Preferences.
Malformed admitted Preference payloads are internal errors rather than skipped rows.

The pure `task_priority` module merges indifferent Tasks, then finds strongly
connected components in the preferred-to graph. Each cycle (including a strict
edge within an indifference group) is merged for this request and emits one
`preference_cycle` diagnostic listing its sorted Task IDs and asking for resolution.
Planning continues. A node's bucket is its longest-path depth from the best layer.

For bucket position `p` and layer count `m`, request values are:

| Case | Value |
|---|---|
| Unranked (takes precedence) | 0.1 |
| One ranked bucket, or `p = 0` | 1.0 |
| `p = m - 1` with multiple buckets | exactly 0.1 |
| Interior bucket | `1.0 - 0.9 * p / (m - 1)` |

The lowest endpoint is assigned explicitly to avoid rounding below 0.1. Both
kernel conversion sites send `priority: 1.0`, so Stage 3 does not count rank twice.
Caller-supplied request values default to 1.0 and must be finite and nonnegative.

Kahn's ready set orders by `(priority_order, latest_finish, task_id)`. Ranked Tasks
use bucket `p`. Unranked Tasks use `m - 1` when `m >= 2`, and otherwise `m`.
The deadline is the allowed range's latest finish; Tasks without a range use
`u64::MAX`. With neither Preferences nor ranges this preserves Task-ID ordering.

`task_priorities` is a response-only explanation sorted by Task ID. It contains
bucket, bucket count, normalized rank (`p / (m - 1)`, or 0 for one bucket), and
value. Unranked Tasks have no bucket or normalized rank. It is never persisted
in the Plan; caller-supplied requests omit the empty explanation.

The default chunked sweep (`UBU-D0279`/`UBU-D0280`) rescues the case where
priority-first placement would let a wide-range Task occupy the only slot of a
narrow-range Task. It compares value-first, most-constrained-first, and
value-density fills and retains the greedy benchmark as a candidate.
`UBU_PLANNER_STRATEGY=greedy` restores the earlier first-fit behavior.

Known limitation: packing failures that no fill rule or look-ahead resolves still
fail the whole Plan until partial placement (`UBU-Q0155`). This does not add
Preference capture, routine inheritance, Objective-derived value, or learned
priorities.
