# Declared routine requirements

An id names one routine, but “after a meal” can mean breakfast for one brush
and dinner for another. Quick UbU routines can carry the targets they
`establishes` and the targets they `requires`, alongside explicit `after`
edges. Each requirement names a UniverseState target in its `fact` field:

```json
{"establishes":["facts.teeth_clean"],"requires":[{"fact":"facts.fed","maximum":[10800,0]}]}
```

Durations use Quick UbU's `[seconds, nanos]` convention. Omitted `offset` means
zero; omitted `maximum` means unbounded. Quick UbU preserves these declarations
through `routine-import` → `snapshot`; its own scheduler ignores them.

## Choosing a predecessor

At import, consider other routines that survived `routine_payload`, declare
the exact required target, have a parseable start and whole-second duration,
and finish nominally at or before the requiring routine's start. The routine
with the latest nominal end wins. Compare local wall-clock seconds, with no
previous-day search. Equal greatest ends are ambiguous: skip the edge rather
than break a tie by id. A tie at an earlier end does not matter if a later
unique candidate exists.

That nearest-preceding rule makes three meals tractable without another
selector vocabulary. With Breakfast, Lunch and Dinner establishing `facts.fed`,
and only Brush night establishing `facts.teeth_clean`, the example resolves:

| Requiring routine | Establisher | After bounds (seconds) |
| --- | --- | --- |
| Brush morning (08:15) | Breakfast | [0..1800] |
| Brush night (22:00) | Dinner | [0..10800] |
| Sleep | Brush night | [0..1800] |

Resolution happens at import. Mainline stores only an ordinary `objective_id`
after edge with `minimum_seconds` and, if bounded, `maximum_seconds` (never
null). Resolved and explicit edges to the same predecessor merge using the
largest minimum and smallest maximum. Nothing downstream distinguishes the
origin of the edge. The import response's `resolved` list is therefore the only
record of the choice: one item per successful requirement with `quick_ubu_id`,
`target`, `establisher_quick_ubu_id` and `establisher_title`, sorted by
`(quick_ubu_id, target)`. Save that response when troubleshooting a choice.

## Import failures

Each failure skips only the offending edge; the routine and its other valid
edges still import. Self-establishment cannot satisfy a requirement. Routines
skipped by `routine_payload` cannot establish anything for resolution.

| Condition | Reason |
| --- | --- |
| Invalid UniverseState target | `requirement_invalid_target: <target>` |
| No eligible other routine establishes it | `requirement_unestablished: <target>` |
| Establishers exist, but none ends at or before this routine starts | `requirement_unestablished_before: <target>` |
| Two or more tie for the greatest eligible nominal end | `requirement_ambiguous: <target>` |
| Fractional offset or maximum | `requirement_fractional_bound: <target>` |
| Maximum below offset | `requirement_inverted_bounds: <target>` |

The target grammar is a documented local copy of
`ubu-schemas/schemas/core/precondition.schema.json`'s pattern because core's
parser is private:

```text
^(facts|numeric_values|set_memberships|event_markers)\.[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)*$
```

Bounds validation precedes candidate selection after target validation. Core
requires nonnegative minima, so a negative offset also reports inverted bounds.
If merging would produce a maximum below the combined minimum, that requirement
reports inverted bounds and leaves earlier edges intact. These unspecified
cases follow the requirement that a bad edge must not discard its routine.
The prescribed ambiguous reason reports the target, not candidate identities;
the specified response shape has no field for tied candidates.

## This is not precondition checking

The ticket supplied this prototype result at `8f003d1`: Brush establishes clean
teeth, Dinner clears that fact, and Sleep needs it. A dependency alone held,
while replay showed the precondition violated when Sleep ran:

```text
PROBE[pinned] status="ok"
PROBE[pinned] link task_…6e02 <- task_…6e01
PROBE replay 2026-09-24T09:00:00Z Brush and floss   precondition=—
PROBE replay 2026-09-24T18:00:00Z Dinner            precondition=—
PROBE replay 2026-09-24T22:33:20Z Sleep             precondition=VIOLATED
```

status="ok" on a violated plan is worse than a visible block.

This is supplied evidence, not a probe rerun against private data. Declarations
select the target of an ordinary edge; they neither simulate UniverseState nor
implement threat resolution, probabilistic effects or numeric accumulation.

## Authoring bounds

A requirement's maximum must span the gap implied by both routines' own nominal
starts. If Brush ends nominally at 21:35 but Sleep cannot start until 22:30,
a thirty-minute maximum is impossible. The existing instantiator reports
`routine_after_maximum_infeasible`; the saved `resolved` list identifies which
edge needs attention. Broad `establishes` declarations cost nothing until
another routine requires the target.

## Known limits

1. **This does not check preconditions.** A routine that establishes a target and one that destroys it are both invisible to the planner. `requires` orders work; it does not verify that the target is still true when the Task runs. The bounded chain from P1B-24 is what keeps an interloper out of the gap, and it does so by leaving no room, not by reasoning about state.
2. **Resolution is by nominal time, not realized time.** The establisher is chosen from declared `start_time` and `duration` at import. If a day actually runs so late that a different establisher would have been nearer, the edge does not move.
3. **Same-day only.** Nominal ends are compared as local wall-clock seconds within one day, so a routine just after midnight cannot resolve to an establisher from the previous evening. The existing `routine_after_unmatched` diagnostic still covers a predecessor whose recurrence does not land on the same date.
4. **The declarations live in Quick UbU.** `RoutineInstanceTemplate` has no `effects` field and occurrence Tasks carry none, so mainline has nowhere to record what a routine establishes. Until a mainline authoring surface exists, `routine.json` is the only place these can be written. The separable follow-up is to carry `establishes` onto the template and into the occurrence payload so that completing a routine actually mutates `UniverseState` through the existing `apply_completed_effects` path — that one is a `ubu-core` change and brings the seven-repo chain with it.

