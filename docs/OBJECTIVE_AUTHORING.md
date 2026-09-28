# Objective and routine authoring

An Objective is a desired outcome. A routine is not a separate kind of object:
[UBU-D0286](https://github.com/UbU-project/ubu-design/blob/f7c4a1dbe2979fa498ae115bb2ee74e6772d4c2c/DECISIONS.md)
makes a routine an **evergreen Objective carrying `recurrence` and
`routine_instance_template`**, from which Tasks are instantiated directly. The
two fields are a pair. `recurrence` says when; `routine_instance_template`
says what each occurrence is. Authoring therefore has one set of endpoints, and
a routine is written through the same ones as any other Objective.

## The one-way door this closes

Before P1B-38 the only way an Objective could enter the canonical store was a
Quick UbU import. Mainline had no `POST /objective` and no routine authoring;
`GET /routines` returned streak summaries, not definitions. Quick UbU has
`ObjectiveAdd` and `RoutineImport`, so the first morning that needed a new
routine sent the operator back to the tool he was switching away from, to
author it there and import it again. The structure of the day could only be
born outside mainline. `POST /objective` and `PATCH /objective/:objective_id`
let it be born, and changed, here.

## API

Both writes require `schema_version: "ubu.orchestrator.objective.v1"`. Unknown
fields are rejected.

- `POST /objective` accepts `title`, and optionally `description`, `priority`,
  `mode`, `recurrence` and `routine_instance_template`. It returns HTTP 201
  with `schema_version`, `objective_id` and `version`. The Objective is
  admitted `active`.
- `PATCH /objective/:objective_id` requires `expected_version` and accepts any
  of `title`, `description`, `priority`, `status`, `recurrence` and
  `routine_instance_template`. An explicit `null` clears an optional field; an
  absent field is left as stored. A stale `expected_version` returns HTTP 409
  with `version_conflict` and writes nothing, in the same shape as
  `PATCH /task/:task_id`. `mode` is not editable.
- `GET /objectives` lists every Objective as `objective_id`, `title`,
  `status`, `priority`, `mode`, `is_routine` and `version`, in creation order
  then id. It is a plain read with no paging.
- `GET /objective/:objective_id` returns the full canonical payload with its
  `version` and `is_routine`. For a routine this is the definition — the
  `recurrence` and `routine_instance_template` that the streak summary at
  `GET /routines` does not carry. An unknown id returns HTTP 404 with
  `unknown_objective`.

| Code | Reason and recovery |
| --- | --- |
| `objective_missing_title` | The title is absent or empty. Supply one. |
| `objective_routine_fields_incomplete` | Exactly one of `recurrence` and `routine_instance_template` is present. Supply both, or neither. |
| `objective_routine_requires_evergreen` | Routine fields are present and `mode` is not `evergreen`. An absent `mode` means `one_time`. |
| `objective_invalid` | The assembled Objective is rejected by `ubu_core::core::Objective`, or `priority` is outside 0–100. The message is the core error. |
| `unsupported_objective_field` | The edit names a field outside the allow-list. |

The first three are checked in that order, before anything else. Every
authored payload then round-trips through `ubu_core::core::Objective` before
admission, exactly as the importer's `normalize` does, so the orchestrator
never admits an Objective the core type would reject. Core's own rules arrive
as `objective_invalid`; notably a routine may not carry a `priority`
([UBU-D0288](https://github.com/UbU-project/ubu-design/blob/f7c4a1dbe2979fa498ae115bb2ee74e6772d4c2c/DECISIONS.md):
occurrences are mandatory and carry no priority), and a template may not name
its own Objective in `after`.

Admission goes through the ordinary `queries::admit_object` path. Authoring is
serialized with Quick UbU imports and with routine materialization.

## Why native authoring omits `provenance.source`

A natively authored Objective carries
`provenance: {created_at, authority_source: "user"}` and nothing else. The
client cannot supply provenance, an id, or either version counter.

The importer writes `source: {source_kind: "quick_ubu", source_id: …}` on
everything it creates, and its reconciliation reads only rows whose
`source_kind` is `quick_ubu`. That scope is what it treats as its own: it
matches those rows to snapshot entries by `source_id`, rewrites the ones that
differ, and reports the ones that no longer appear as stale.

If native authoring set a `quick_ubu` source, the next import would adopt the
Objective as one of its own: it would either rewrite it to match whichever
snapshot entry shared the `source_id`, discarding the operator's edits, or find
no such entry and report it as stale. Omitting `source` keeps the Objective
outside that query entirely. The same reasoning governs captured Tasks
(P1B-27) and native Preferences (P1B-36). It is verified directly: a native
Objective and a native routine, the routine sharing a title with an imported
one, are byte-identical and at the same version after two imports.

## The two version counters

A routine carries two counters, and the rule for each is shared with the
importer (`quick_ubu_import.rs`, the block beginning
`if object_type == ObjectType::Objective`).

| Counter | Lives in | Bumps when |
| --- | --- | --- |
| `schedule_version` | `recurrence` | anything else in `recurrence` changed |
| `template_version` | `routine_instance_template` | anything else in `routine_instance_template` changed |

Each object is compared with its counter removed. If the remainder is
unchanged the counter keeps its value; if it differs the counter increases by
exactly one. The two are independent: a template edit leaves
`schedule_version` alone, a recurrence edit leaves `template_version` alone,
and an edit that changes neither — a rename, or both objects restated exactly
as stored — bumps neither. Both start at `1` on creation.

The comparison is made on the canonical form, after the round trip through the
core type, so restating a default (`occupies_capacity: true`) or sending a
counter value is not a change. Counters are server controlled: a value supplied
by the client is overwritten on create and on edit.

Because the rule is the importer's, an edit made in the app and the same edit
arriving by import produce the same versions for the same change. Both
counters are part of each occurrence's key
(`<objective>/s<schedule_version>/<date>T<nominal_start>/<placement>/t<template_version>`),
which is how a materialized occurrence records the definition it came from.

`recurrence` is replaced whole. Per-date `overrides` and `exdates` live inside
it, so an edit that supplies `recurrence` must restate the ones it means to
keep; `GET /objective/:objective_id` returns them. The importer behaves the
same way.

## A template edit does not touch materialized occurrences

Editing a routine changes its definition, not the Tasks already created from
it. Occurrences admitted for today keep the template they were born with, and
their key still names the `template_version` they were created under. The
change takes effect at the next `materialize`. When an edit changed the
template, the response says so in `notice`:

> The routine template changed. Occurrences already materialized keep the
> template they were created with; the change applies at the next materialize.

## Overlapping routines are rejected when written

P1B-38 left native routines unchecked, and measured the consequence: two
native Static routines at the same hour were both admitted, and every later
`POST /import/quick-ubu` was then refused until one was moved. Since P1B-39
the overlap is refused at the write, while the operator still remembers what
they meant.

`POST /objective` and `PATCH /objective/:objective_id` run the importer's own
check, `static_overlaps`, over the live routines plus the routine being
written, across the same window the importer uses: now to 366 days from now.
A pair the importer would reject is a pair authoring rejects.

- The rejection is HTTP 400 with one `objective_routine_overlap` diagnostic
  per conflict. Each names both routines by Objective id and title with their
  local windows, the first colliding local date, and whether it is a
  `Conflict with another routine` or a `Conflict with itself`. Nothing is
  written.
- An edit is compared against the other routines and its own *new*
  definition. Its stored definition is left out, so moving a routine onto the
  window it used to occupy is allowed.
- A routine whose own occurrences collide, such as a daily routine lasting
  longer than a day, is rejected like any other overlap.
- An overlap that exists only across a daylight-saving shift is exempt, as it
  is at import.
- Only conflicts involving the routine being written are reported. An overlap
  between two other routines is not this write's to answer for, and does not
  block it.
- A write that leaves the Objective `satisfied` or `abandoned` is not
  checked, so withdrawing a routine always succeeds.

The import-time check remains as the backstop for anything that arrives by
import or was admitted before this check existed. When it names a native
routine it says so, and points at `PATCH /objective/:objective_id` rather
than `routine.json`.

## Status, and why there is no delete

Objectives are not deletable. Unlike a Preference, an Objective has work
hanging off it and a history of serving it. `PATCH` may set `status`, and the
existing vocabulary is the whole mechanism: `open`, `active`, `satisfied`,
`abandoned`. Withdrawing an Objective is setting it to `abandoned`. The
Objective vocabulary has no `moot`; that word belongs to Tasks.

The store row's status follows the payload: `open` and `active` keep the row
`active`, so that a routine stays live, and `satisfied` or `abandoned` are
written to the row as well. A withdrawn routine stops materializing, and a
withdrawn imported routine is reported by the importer as diverged rather
than being rewritten.

## Reading Tasks

P1B-38 also adds the Task reads that any editing surface needs.
`TASK_READ_SCHEMA_VERSION` is `ubu.orchestrator.task_read.v1`. A read has no
body, so `schema_version` is an optional query parameter; when supplied it
must match, and anything else is `unknown_schema_version`.

- `GET /task/:task_id` returns the canonical payload, its `version`, `status`
  and `is_routine_occurrence`. An unknown id returns HTTP 404 with
  `unknown_task`.
- `GET /tasks?status=` lists `task_id`, `title`, `status`, `version`,
  `placement`, `duration_estimate`, `due_at`, `objective_id`, `category_tag`,
  `is_routine_occurrence` and `container_id`, ordered by status, then creation
  time, then id. `status` defaults to `active` and accepts `active`,
  `completed`, `failed` and `moot`; anything else is `unknown_task_status`.
  Absent optional fields are omitted. `placement` is `static` when the Task
  has a `static_window` and `planned` otherwise. `container_id` is present
  only for a child of an active Container.

A routine occurrence is visible in the list and marked, but
`PATCH /task/:task_id` still rejects it with
`routine_occurrence_not_editable`. The mark lets a client render it read-only
instead of discovering the rejection on submit.

## Known limits

1. **No `target_date` on Objectives.** Quick UbU has one; the schema does not, and adding it is a six-repo chain. Deadlines live on Tasks as `due_at`.
2. **Objectives cannot be deleted**, only mooted. Deliberate, and unlike Preferences.
3. **A template edit does not reach today's occurrence.** It applies at the next `materialize`.
4. **`GET /tasks` does not paginate** and filters only by status. No text search, no category filter, no date range.
5. **The UI covers four routes of fifty-one.** Preferences, decomposition, Containers, routines, the occurrence override, the advisory queue, the Google Calendar chain and Quick UbU import all remain unreachable from the app. Later tickets.
6. **`ubu-ui` still wires the old generic `/projection/*` routes**, not the `/projection/calendar/*` chain from P1B-29…35. Both exist server-side, so nothing is broken, but the app is driving the older projection path.
7. **No routine authoring in the UI.** §B makes it possible over HTTP; the screen comes later.
8. **No Task delete**, unchanged from P1B-27.

The limits above are recorded verbatim from the ticket. Two of them use words
the implementation measures differently: "mooted" in limit 2 is the Objective
status `abandoned`, and "fifty-one" in limit 5 is 49 path keys carrying 53
operations. See `P1B-38_VERIFICATION.md`.
