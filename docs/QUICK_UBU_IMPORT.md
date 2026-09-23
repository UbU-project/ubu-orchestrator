# Quick UbU import

Produce a snapshot without modifying the source store:

```sh
quick-ubu --store /path/to/quick-ubu.db snapshot /path/to/snapshot.json
quick-ubu snapshot /path/to/snapshot.json --from-json /path/to/legacy-store.json
```

POST `/import/quick-ubu` with `{"snapshot_path":"/path/to/snapshot.json",
"timezone":"America/New_York","dry_run":false}`. The server reads that local
path. The timezone defaults to America/New_York and is checked for shape only;
planning resolves real timezones and materializes occurrences as described in
[ROUTINES.md](ROUTINES.md).

| Source | Mainline |
|---|---|
| Routine | Active evergreen Objective with recurrence and routine instance template |
| Daily, Weekly, MonthlyDay, MonthlyFirstWorkday, QuarterlyFirstWorkday | Corresponding daily, weekly, monthly_day, first_workday_of_month, first_workday_of_quarter rule |
| Routine start_time, duration | Template nominal_start and fixed duration_estimate |
| Routine category, reminders | Template tags/category_tag and nonnegative reminder_minutes |
| Routine dynamic | Planned template with start_time–latest_tod range; absent latest_tod becomes 23:59:59 |
| Routine not dynamic | Static template without a local range |
| Routine transparent | Inverse occupies_capacity |
| Routine after | Reference to mapped Objective id plus nonnegative offset_seconds |
| Backlog, Scheduled, Active, Deferred Task | Active Task |
| detail, tags/category, pinned, due | description, union of tags with category_tag, static_window, due_at |
| Positive est_duration, transparent | Fixed duration_estimate, inverse occupies_capacity |
| Both ordered range endpoints on unpinned Task | allowed_time_range |
| blocked_by | References to mapped Task ids |
| Strict / Indifferent singleton-bundle Preference | a_preferred_to_b / a_indifferent_to_b Task Preference |

The response reports created, updated, and unchanged counts by kind, the count of
Quick UbU objectives not imported, skipped items/fields, diverged objects, and
stale objects. `schema_version` is `quick-ubu-import/1`. No occurrence Tasks are
created or planned here. Logs, completion history, calendar links, colors,
clarification/decomposition state, tiers, affect costs, skills, objective links,
deferral/commitment metadata, and task-level reminders are not imported.

| Reason | Treatment |
|---|---|
| routine_occurrence, orphaned_routine_occurrence | Skip Task; planning derives mainline occurrences |
| completed, past_static_window | Skip Task |
| invalid_local_range | Skip routine |
| negative_reminder | Drop reminder, retain routine |
| after_reference_missing, after_self_reference, negative_after_offset | Drop routine after entry |
| partial_allowed_range, invalid_allowed_range | Drop Task range |
| dependency_not_imported | Drop Task dependency |
| task_after_unsupported | Drop Task after metadata |
| non_singleton_bundle, task_not_imported | Skip Preference |
| invalid: … | Skip an item failing core validation; fractional-second durations are also skipped rather than truncated |

Fractional-second after offsets are individually dropped and reported as
`invalid: fractional after offset is not representable in whole seconds`.
Missing/malformed files, unsupported snapshot versions, mismatched source IDs,
and missing Task origins produce HTTP 400 `invalid_quick_ubu_snapshot`. A malformed
timezone shape produces HTTP 400 `invalid_timezone`.

Identity is `(object_type, provenance.source.source_kind = quick_ubu, source_id)`.
Routine/Task source IDs are their Quick UbU IDs; Preference IDs use the ordered
`left_bundle_id|right_bundle_id` pair. Imported objects use the `quick-ubu-import`
compartment and user authority. The server compares canonical mapped payloads,
preserving existing IDs and provenance creation time, and Preference acquired_date
and enabled state. Identical imports write no objects or mutation envelopes.
Recurrence edits increment schedule_version only; template edits increment
template_version only. Prior versions are otherwise retained.

Mainline status wins: an existing object whose store status is not active is
reported in `diverged` (the `mainline_diverged` classification) and is never updated.
Previously imported objects absent or skipped in this run are `stale`; they are
reported and never mutated. Dry runs perform mapping and comparison but write
nothing. Dropped-field reports can recur on an otherwise unchanged import.
Repeated Preference source pairs map once, in source order; subsequent entries
are reported as `invalid: duplicate preference source identity`. Concurrent
imports sharing the application state are serialized to preserve source identity.

## Routine overlap gate

Before mapping Tasks or Preferences or writing anything, the importer evaluates
the routine set the snapshot would leave live over 366 days from the planning
clock's now. Two capacity-occupying Static occurrences overlap if their intervals
intersect; touching endpoints are allowed. Planned ranges, transparent Statics,
and one-off commitments are not checked. Instantiation diagnostics never reject;
unknown zones and unevaluable Objectives are unchecked. Enablement windows that
open beyond this one-year span are not covered.

The gate includes live routines absent from this snapshot: import reports those
as stale but never retires them. To fix an overlap with one, put that routine back
in the snapshot at a non-overlapping time. Mapped routines replace stored ones by
Objective ID; a mainline-diverged inactive row is never replaced or reactivated.

Overlaps caused only by a DST shift are exempt when both schedules resolve to the
same zone and their nominal local intervals never overlap. Those intervals use
the already-after-lowered local start plus scalar duration, not the template's
original anchor. A single ordinary local collision makes the pair an error.
Cross-zone pairs are never exempt. Overnight and self-overlaps are checked.

Rejection is HTTP 400 with `overlapping_routines` causes grouped by connected
routines, local windows, first collision date, pair counts, and date counts.
At most 25 groups are shown, followed by an omitted-group count when needed.
The entire file is rejected without writes, including under `dry_run`. Stagger
start times, shorten a routine, or make it transparent before retrying.
