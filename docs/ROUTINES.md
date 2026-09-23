# Routine occurrences

Planning materializes each live routine Objective before reading Tasks. A live
routine has an active store row, payload status `open` or `active`, a recurrence
schedule, and a routine instance template. Daily, selected weekdays, selected
month days, first workday of a month, and first workday of a quarter are supported.
Workdays are Monday–Friday. Enablement bounds are inclusive and EXDATEs suppress
dates. Expansion evaluates local dates from one day before the horizon's start
through one day after its end. Creation keeps only windows intersecting the
remaining horizon; date-specific diagnostics cover the unpadded local dates.

Times use the schedule's IANA timezone, resolved by chrono-tz. An ambiguous local
time uses the earlier UTC instant and emits `routine_occurrence_ambiguous_local_time`.
A nonexistent nominal start or range endpoint suppresses that occurrence with
`routine_occurrence_nonexistent_local_time`. This fixed, documented policy is the
recorded DST disambiguation required by UBU-D0276. Unknown zones emit
`routine_timezone_unknown` and preserve existing occurrences. Invalid Objective
payloads emit `routine_objective_invalid` and likewise protect their cache.

## Relative placement

Definitions are evaluated predecessor first, with Objective-ID tie breaking.
Cycle members and every downstream definition have their `after` references
ignored, with one sorted `routine_after_cycle` diagnostic. Same-date predecessors
lower Static starts or planned range starts against nominal end plus offset.
Duplicate predecessor references use the largest offset and one Task edge.
Unmatched references contribute no floor and emit `routine_after_unmatched`.
An impossible lowered range emits `routine_after_infeasible` and creates nothing.

Quick UbU floors against the predecessor's placed end, then drops the floor when
it completes. Mainline uses nominal lowering plus a realized-end floor: a
completed same-day predecessor pushes a planned dependent to its stored
`updated_at + offset`. This is a deliberate deviation, without minimum-lag kernel
edges. Nominal lowering alone does not space the three check-ins: their nominal
ends plus one hour already precede the next ranges. Completion at 11:40 therefore
pushes the noon check-in to 12:40. Unmatched references are also a deviation:
Quick UbU leaves the dependent unplaced; mainline ignores the unmatched floor.

Lowering always uses current definitions, including on held days. Dependency
edges and realized floors instead resolve the day's stored non-superseded Task,
including completed history under an older key. Completing a predecessor and
then editing its template does not disconnect its successors.

## Lifecycle and history

Occurrence keys include Objective ID, schedule version, local date, the template's
own nominal-start string, placement, and template version. IDs are allocated
before payloads, so Tasks created together can name their predecessors. Identical
materializations write nothing. A changed derived payload can refresh an active,
unlogged future occurrence in place, retaining provenance. Routine compartment
labels are inherited; creation and lifecycle maintenance use system authority.
Reminders remain template metadata and are not copied to Tasks.

An active occurrence whose Static end or allowed-range end has passed becomes
`failed` and receives a `task_failed` Log with `routine_outcome: missed`. This
applies even if it was started or otherwise logged. `POST /task/:id/action` with
schema version `ubu.orchestrator.task_action.v1` supports:

- `skip`: active or missed occurrences become `moot`/`user_declared_moot` under
  user authority, with a `decision_recorded` Log recording `occurrence_skipped`.
- `complete`: active Tasks and missed occurrences become completed. Late
  completion corrects a miss; failed non-occurrence Tasks remain ineligible.
- Existing `snooze` and `override` retain their existing meaning.

The legacy start/done/snooze/reject/decompose routes reject occurrences with
`use_recorded_action`; their log-only behavior cannot settle an occurrence.
Non-active occurrences are never frozen into a repair Plan. Active occurrences
frozen by a user override retain that protection.

Future keys replaced by edits, retired Objectives, or newly infeasible lowering
become `moot`/`superseded`, with a `task_moot` outcome Log. Retirement includes
inactive rows, satisfied/abandoned Objectives, or removal of recurrence/template.
Unevaluable Objectives do not retire their occurrences. Any Log naming an active
occurrence is execution evidence, except recalculation requests and the
materializer's own `routine_outcome` Logs. Such occurrences remain active and emit
`routine_occurrence_edit_conflict` instead of being superseded.

A completed, skipped, missed, or protected active occurrence under a different key
holds its Objective/date: no replacement is created or revived. Superseded history
does not hold a day. When a superseded key becomes eligible again, the same Task
ID is revived with provenance retained and `moot_reason_code` removed. Revival
writes a version, not a new outcome Log. Per-occurrence validation/admission
failures emit `routine_occurrence_write_failed` and do not fail planning. Import
and materialization locks are acquired in that order; user-action races are
handled by version preconditions. Materialization reads active occurrences and
history only within each evaluated date range.

## Committed time and triage

Planned occurrences always occupy capacity, including templates marked transparent.
For them, `occupies_capacity: false` is projection metadata only. Transparent
Static occurrences remain direct non-capacity steps. A planned occurrence keeps
its complete allowed range; the planning end extends to cover kept ranges so a
rolling horizon does not truncate tomorrow's first check-in.

Overlapping capacity Statics involving any occurrence form a committed-time
cluster. Each member remains on the Calendar at its true window. The kernel sees
one carrier reserving the union, including every external prerequisite of cluster
members. Covered steps keep `occupies_capacity: true`; every candidate and saved
Plan restores the carrier's true window. Warnings identify overlapping routines
(`routine_occurrences_overlap`) and every occurrence sharing a cluster with a
one-off commitment (`routine_occurrence_overlaps_commitment`). Routine-definition
overlap rejection now happens at import over the prospective live set, as described
in [QUICK_UBU_IMPORT.md](QUICK_UBU_IMPORT.md). Planning's `routine_occurrences_overlap`
is the safety net for an error state the importer refuses to create; direct store
admission, pre-existing state, and DST-only collisions can still reach planning.
Two overlapping one-off Statics still cause
`static_task_collision`. Stale precedence edges between Static occurrences are
dropped with `routine_occurrence_edge_dropped`; fixed placements remain intact.

Occurrences never participate in Preference layering or task priorities. Their
value is 0.0; the graph and greedy ready order place them before priority buckets.
See [PLANNING_PRIORITY.md](PLANNING_PRIORITY.md) for the chunked-search limitation.

A planned occurrence with insufficient window length, an excluded prerequisite,
or no gap among fixed commitments is excluded with
`mandatory_occurrence_unplaceable`. Its message names the Task first and asks the
user to do it late, skip it, or change other commitments. Moving an occurrence
requires overrides, which are out of scope. These exclusions and overlap warnings
produce medium `routine_triage` findings with `blocking: false`. They do not mark
the Calendar stale or request recalculation.

**Flagged design deviation:** DESIGN §15.2.2 and UBU-D0289 treat inability to place
a mandatory occurrence as blocking. Phase 1b cannot model irregular availability,
so this ticket warns and continues generating a usable Plan instead of blocking
ordinary days over a short routine. This does not implement partial placement or
a kernel-visible mandatory marker.

Known limits:

- Repair Plans store no risk report; warnings are in repair diagnostics until the
  next generate.
- A closing window warns for at most the occurrence's duration before it is missed.
- The no-free-time check examines the occurrence's own window against fixed
  intervals, not kernel dependency release/deadline narrowing. A planned
  predecessor of a Static occurrence can still fail the Plan; the motivating
  routines have no such shape.
- Packing failures among Dynamic Tasks still fail the whole Plan until partial
  placement. The chunked fills can put value-0.0 units after valued units.
- Two planned occurrences with execution evidence can retain stale edges after an
  `after` reversal and form a cycle. Generate returns `cyclic_dependency_graph`
  until one is skipped. Static-to-Static stale edges are dropped instead.

## Read-only rollups

`GET /routines` returns `routine-summary/1`, sorted by title then Objective ID.
Each live routine has done, skipped, missed, and pending counts, its current
streak, and its latest terminal occurrence's local date and outcome (or null).
Completed means done; user-declared moot means skipped; failed means missed.
Active occurrences whose windows have elapsed also read as missed, without
writing their rows or Logs. Other active occurrences are pending. Superseded
occurrences are not counted.

History is ordered newest first by local date then window start. Pending entries
are ignored for streaks and last occurrence; the streak counts leading done
outcomes until a skip or miss. Deliberate reading of UBU-D0286's “Logs that roll
up”: summaries read current occurrence status, mirrored by outcome/action Logs,
so late completion corrects counts without double-counting the earlier miss.
