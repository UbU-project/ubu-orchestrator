# Task capture and editing

UBU-D0275 and DESIGN.md §4.2 define the Phase 1b exit criterion:

> The switch: the dogfooding user's primary daily planning runs on mainline UbU.

That switch needs a way to capture a one-off Task during the day. Mainline now
accepts Tasks directly, without editing a Quick UbU snapshot. The existing
`POST /task/:task_id/decompose` endpoint appends a `Decompose` action to the Log;
it does not create Tasks.

## HTTP contract

Both endpoints require `schema_version: "ubu.orchestrator.task_capture.v1"`.
A missing schema version produces `missing_schema_version`; an unsupported one
produces `unknown_schema_version`, both HTTP 400.

`POST /task` accepts a title and optional editable fields, returning HTTP 201:

```json
{"schema_version":"ubu.orchestrator.task_capture.v1","title":"Call the plumber","duration_estimate":{"type":"fixed","seconds":900}}
```

The response contains `schema_version`, the generated `task_id`, and `version: 1`.
The server admits the validated Task with `VersionRef::Absent` in the
`user-capture` compartment. It owns `id`, active `status`, and user `provenance`:
clients neither generate ids nor forge origin or bypass lifecycle logging.
Missing, null or empty titles produce `missing_title`.

`PATCH /task/:task_id` accepts the same fields plus a required integer
`expected_version`. For example, after capture:

```json
{"schema_version":"ubu.orchestrator.task_capture.v1","expected_version":1,"title":"Call the plumber tomorrow","due_at":null}
```

A successful edit returns HTTP 200 and the next version. Explicit JSON `null`
removes an editable field, including a previously set due date. Removing the
required title is invalid. Both operations deserialize the whole resulting
payload as `ubu_core::core::Task` and call its validator before admission.
Invalid Tasks return HTTP 400 with the underlying validation error.

The shared allow-list is exactly:

```text
title, description, duration_estimate, allowed_time_range, static_window,
due_at, tags, category_tag, occupies_capacity, preconditions, effects,
correlation_groups, blocked_by, objective_id
```

Every other field produces HTTP 400 `unsupported_capture_field`, naming the key.
That includes `id`, `status`, `provenance`, `occurrence`, and `moot_reason_code`.
Edits preserve server metadata: status, compartment, creation time and provenance.
Unknown Task ids produce HTTP 404 `unknown_task`.

## Versions and importer isolation

`expected_version` prevents an edit from silently overwriting another change.
A mismatch returns HTTP 409 `version_conflict`, naming both expected and current
versions, without changing the stored Task. Admission carries the observed
version as a store precondition; a concurrent write between the service read and
admission therefore also returns HTTP 409 through `StoreError::PreconditionFailed`.
Every other store error retains HTTP 500. Missing `expected_version` is rejected
by Axum's ordinary request deserialization (HTTP 422).

Captured provenance is `{created_at: state.planning_now(), authority_source:
"user"}` with **no `source` field**. This is the truthful user origin and the
reason a captured Task survives re-import: the Quick UbU importer scopes its
reads and writes to `provenance.source.source_kind = 'quick_ubu'`. A captured
Task cannot match that query and cannot be superseded, staled or diverged by
those imports. There is no invented `user_capture` source kind.

## Routine boundary and lifecycle

A Task carrying `occurrence` is rejected with HTTP 400
`routine_occurrence_not_editable`, naming its routine as the thing to edit.
This check precedes the version comparison. Materialization rebuilds occurrences
from their routine template; P1B-26 demonstrated the disappearing precondition
when a declaration was put on an occurrence instead of its template.

This surface authors one-off Tasks only. Routine and Objective authoring remain
outside it; routines still use the Quick UbU path. There is no delete or status
PATCH. Existing reject, snooze and done action endpoints own lifecycle behavior
and record Log entries, consistent with UBU-D0254's backend trust boundary.

## Known limits

1. **The version check is advisory.** It reads the current version, then admits. A write landing between the two is caught by the store's own precondition, which is why §B maps it to `409`. On a single-Device desktop instance the window is not reachable in practice; it is named because Phase 2 sync makes it real.
2. **No capture surface for routines or Objectives.** They still come from `routine.json` through Quick UbU, so `establishes`, `requires` and recurrence remain unauthorable in mainline.
3. **No delete.** A captured Task can be rejected, snoozed or completed through the existing action endpoints, but there is no way to remove one that should never have existed.
4. **No UI.** This is an HTTP surface. `ubu-ui` has no capture affordance, so until it does, capture during the day means an HTTP call.
5. **`blocked_by` and `objective_id` are editable but unvalidated as references.** Nothing checks that the named Task or Objective exists; a dangling `blocked_by` is handled downstream by the existing dependency machinery, not rejected here.

