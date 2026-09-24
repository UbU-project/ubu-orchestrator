# Bounded routine after relationships

An unbounded `after` promises order and a minimum delay. It does not promise
proximity. The ticket supplied this probe at orchestrator `64730a8`:

```text
PROBE[explicit-dependency] step 2026-09-24T09:00:00Z Brush and floss
PROBE[explicit-dependency] step 2026-09-24T16:40:00Z Sleep
```

The dependency held despite the roughly seven-and-a-half-hour separation
(7 hours 40 minutes in the timestamps). A maximum expresses the missing
requirement: Sleep must start within ten minutes of the preceding pill Task.
These are supplied illustrative observations; no real Quick UbU data was read
for this ticket.

## Two bounds on the successor's start

```json
{
  "objective_id": "obj_018f3c8e9b2a7c4d8f1e2a3b4c5d6e80",
  "minimum_seconds": 0,
  "maximum_seconds": 600
}
```

Both bounds measure the successor's **start** relative to the predecessor's
**end**. `minimum_seconds` is required and nonnegative. `maximum_seconds` is
optional and must be at least the minimum. Absent maximum means order without
a proximity ceiling. A finish bound could not express a thirty-minute Sleep
starting within ten minutes: it would reject the entire duration instead.

Across predecessors, the largest minimum-derived floor and the smallest
maximum-derived ceiling win. Duplicate references merge the same way: largest
minimum, smallest present maximum. An unbounded duplicate does not erase a
bounded one. Ordinary `blocked_by` dependency edges remain unchanged.

## Lowering and actual execution

Instantiation uses each matched same-date predecessor's nominal end. It lowers
`nominal_end + minimum` into the successor's earliest time, and
`nominal_end + maximum + successor_duration` into its latest finish. The latter
is intersected with the declared latest finish. The duration is the template's
scalar duration; this ticket does not add a bounded edge to the kernel.

The ticket's example is a one-minute predecessor ending at 21:01 and a
thirty-minute successor:

| | `minimum_seconds: 0` alone | `minimum_seconds: 0, maximum_seconds: 600` |
| --- | --- | --- |
| Sleep's allowed range | 21:01 → **23:59** | 21:01 → **21:41** |

When the predecessor completes, its actual completion time supplies a realized
floor and, if declared, a realized ceiling. The ceiling is
`actual_end + maximum + successor_duration`, clamped to `Occurrence.declared_end`:
the latest finish **before** any nominal maximum was applied. Multiple realized
ceilings take their minimum.

Planning **assigns** that already-clamped ceiling to `window.end`. It does not
intersect with the old nominal ceiling, because a late predecessor must be able
to move the window later. Intersecting with the lowered `Occurrence.end` retains
a stale bound and can close the window entirely. An early completion can move
the ceiling earlier too. The existing realized floor handling remains intact.

## Diagnosing contradictory ranges

- `routine_after_maximum_infeasible`: the successor would fit after applying
  the floor, but its maximum makes the allowed range too short. Its message is:
  `Routine <id> on <date> cannot start within maximum_seconds of its predecessor and still honour its own allowed range`
  (ids and `maximum_seconds` are backtick-delimited on the wire).
- `routine_after_infeasible`: the successor would already fail without the
  maximum, such as a minimum pushing it beyond its own latest finish.

Either code suppresses a Dynamic occurrence at instantiation. A Static start
past its maximum emits the new diagnostic but retains its fixed placement.
Static commitments are not silently moved to satisfy a ceiling.

## Preconditions do not create ordering

Preconditions are evaluated against the current UniverseState. Effects apply
only upon real completion; planning does not simulate them to introduce new
ordering edges. The ticket supplied this second probe:

```text
PROBE[effect-precondition] status="ok"
PROBE[effect-precondition] step 2026-09-24T09:00:00Z Brush and floss
PROBE[effect-precondition] blocked=[{"precondition":{"expected":true,"predicate":"equals",
  "target":"facts.teeth_clean"},"task_id":"task_…6e02"}]
```

Sleep was gated out of the day, not ordered after brushing. `after` is the
routine relationship used for ordering until Phase 3; ordinary explicit Task
dependencies still order work. Preconditions/effects are not a substitute.

## Authoring and import

Quick UbU retains `offset` and adds optional `maximum`, both serialized Duration
pairs such as `[600,0]`. Its `routine-import` → `snapshot` path preserves the
maximum; its scheduler ignores it. Mainline import maps `offset` to required
`minimum_seconds`, and emits `maximum_seconds` only when present, never null.
The removed mainline name `offset_seconds` is rejected by schema and core.

Fractional maxima are skipped with
`invalid: fractional after maximum is not representable in whole seconds`;
a maximum smaller than the offset is skipped with `inverted_after_bounds`.
As with existing invalid after references, this skips the invalid edge while
retaining the routine and any valid edges.

An unchanged Quick UbU offset behaves as before. Author maxima that are
compatible with the routine's declared start/range; otherwise the new
infeasibility diagnostic identifies the conflict. The tests inspect the
kernel-visible request window, not randomized Plan placement.

## Known limits

1. **The ceiling does not travel with the predecessor inside a rollout.** It is an absolute window bound, fixed when the request is built. If the predecessor samples long, the successor's start is pushed past a ceiling that did not move with it, and the day reads less feasible than a true bounded edge would make it. This **overstates** risk, which is the safe direction, and it is small where it matters — the Tasks that need tight maxima are short ones. Fixing it means a bounded edge in the kernel contract and the `UBU-D0283` worker: judgment call 3.
2. **A start ceiling is carried as a finish bound.** `window.end = latest_start + duration` is exact only while the sampled duration equals the planned one. A `UBU-D0285` overrun of the successor itself can therefore read infeasible even though its start was inside the bound. There is no start-ceiling channel for Dynamic work to put it in.
3. **An occurrence dropped at instantiation is not `unplaced_tasks`.** Both infeasibility codes return `None` from `expand`, so the Task is never created and `UBU-D0289`'s partial-placement machinery never sees it — including for a mandatory routine, which simply vanishes from the day with a diagnostic. That is pre-existing behaviour of `routine_after_infeasible`, not introduced here, but a second code now reaches it.
4. **Quick UbU does not honour `maximum`.** It carries the field and plans as it always did. Until the switch, the two tools will disagree about any routine that declares one.

