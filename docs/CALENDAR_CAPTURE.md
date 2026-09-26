# Calendar capture

P1B-32 closes P1B-31's known limit 3:

> **Foreign events are not planning constraints.** A real meeting on the calendar is reported and ignored. The planner still does not know the operator is busy then.

This is the capture half of `quick-ubu/gcal`'s `import_from_calendar` at `9ccc8b8`. Its first branch imports a commitment. The later branches use calendar edits as interaction: colour completes or reopens a Task, dragging moves it, resizing changes its estimate, and deleting can remove it. Completion, reopening, moving, re-estimation and their Log/lifecycle evidence are deferred to P1B-33. Mainline reports disappearance and retains the Task.

## Why origin identity matters

A new Task gets a mainline Task ID, allocated at admission. Deriving a new Calendar ID from it would create a duplicate:

```text
PROBE[naive] origin event id  = 5n0q8c9h7g4k2m1p3r6t8v0a2c
PROBE[naive] derived event id = 018f00000000000000000000000000aa
PROBE[naive] first apply of the captured Task:
PROBE[naive] CREATE Dentist (id=018f00000000000000000000000000aa)
PROBE[naive] the operator's original `5n0q8c9h7g4k2m1p3r6t8v0a2c` is untouched and still on the calendar
PROBE[naive] => two `Dentist` events, and rescheduling only ever moves UbU's copy
```

The provenance source carries the existing event ID instead:

```text
PROBE[origin] external id used = 5n0q8c9h7g4k2m1p3r6t8v0a2c
PROBE[origin] unchanged captured Task:     operations: none
PROBE[origin] rescheduled captured Task:   UPDATE Dentist (id=5n0q8c9h7g4k2m1p3r6t8v0a2c)
```

`external_id_for(task_id, Some(origin))` validates and uses the origin. An unusable origin returns `None`; it never falls back to a derived ID. Ordinary Tasks still use `external_id(task_id)`. Provenance lookup uses `{source_kind: "google_calendar", source_id: <event id>}`, following `quick_ubu_import`. Quick's UUIDv5 `CAPTURE_NAMESPACE` is not used in mainline.

Capture appends the origin event, paired with its admitted Task ID, to a new applied-set projection result. This is ownership bookkeeping, not a delivery claim: `operation_results` is empty and no Calendar write is made. Later previews diff against that set. Capture, preview, apply and reconcile share the Calendar projection lock.

## Mapping in both directions

| Calendar input | Captured Task | Projection back to Calendar |
|---|---|---|
| `summary` | `title` | `summary` |
| `start.dateTime`, `end.dateTime` | `static_window.start`, `.end` | Scheduled static start/end as UTC RFC3339 |
| `transparency: "transparent"` | `occupies_capacity: false` | `transparency: "transparent"` |
| `transparency: "opaque"` or absent | `occupies_capacity: true` | `transparency: "opaque"` |
| Unique palette `colorId` | Matching `category_tag` | The same palette colour |
| Shared palette `colorId` | No category; `capture_colour_ambiguous` | Preserve the recorded source colour |
| Unmapped or absent `colorId` | No category; no diagnostic | Preserve the recorded source colour or absence |
| Event ID | `provenance.source.source_id` | The original event ID |

The inverse uses the configured `CategoryPalette`, including operator overrides. It never picks an arbitrary category or uses freeform tags. Time is normalized to whole UTC planning seconds on both sides. A span that cannot occupy at least one planning second is skipped with `capture_event_invalid`. Offset spellings do not create spurious updates.

Mirroring these maps keeps capture → generate → preview stable. The applied record retains explicit popup reminders and the source colour even when those are not Task metadata, so preview preserves them. Default or omitted reminders are accepted and represented as no explicit reminders; Calendar defaults are not imported into the Task. A later write uses the existing projection's explicit reminder body. Other source fields are not imported. The wire parser continues to skip malformed or unsupported entries with diagnostics.

## Explicit capture and ownership

```http
POST /projection/calendar/capture
Content-Type: application/json

{"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"mock"}
```

Use `live` with the existing Google credential-path configuration and explicit process enablement for an operator Calendar. The endpoint has the same live availability guards as reconciliation. It returns schema version, integer `captured`, `updated`, `skipped` counts, and diagnostics. It never returns credential paths or tokens. Synthetic tests inject `RecordingCalendarApi`; the default mock client is seeded from the applied set and invents no foreign events.

Only events classified **foreign** by `calendar_reconcile::classify` enter `plan_capture` and admission. Ownership comes from the applied record. `missing`, `drifted` and `unrecorded` events are not candidates. Once captured, a source is owned; UbU's projected work cannot feed back into new Task admission.

The operator approved this interpretation of the ticket's repeat-capture ambiguity: an unchanged, already captured active source counts as `updated` reuse, with no second admission, version increment or projection result. Owned sources changed on the phone count as skipped and do not edit Tasks. A foreign source that already maps to a Task (for example after applied-bookkeeping loss) reuses that Task ID and admits changed fields with the ordinary version precondition. Unchanged payloads are not admitted again. Inactive source Tasks are skipped rather than resurrected. Duplicate source identities in the store cause an error instead of choosing one.

Pure `plan_capture` leaves `task_id` as `None` for new sources; random IDs are allocated only at admission. New payloads have user provenance, source kind `google_calendar`, a static window and capacity flag. They use the ordinary `user-capture` compartment. Existing user metadata outside the captured fields is retained on a source update. Task admissions and the final applied snapshot are separate store writes; retry after interruption recovers the Task through provenance rather than duplicating it.

All-day entries produce `capture_all_day_unsupported`. Invalid IDs are skipped rather than converted to derived IDs. Expanded recurrence instances still have to satisfy this ID contract.

## Bounded observation and disappearance

`CalendarApi::list_events` takes a `CalendarTimeRange`. Google requests include percent-encoded `timeMin` and `timeMax`, with `singleEvents=true` on every page. The recorder applies the same exclusive overlap rule: event start is before the range end and event end is after the range start. Bounds use the planner's existing horizon resolution: latest stored Calendar window if present, otherwise the process planning clock and configured horizon.

Reconciliation compares only applied entries overlapping the requested range. Its stored observation carries the other applied entries separately, and repair preserves them. A bounded response cannot establish that an out-of-range event was deleted.

For an active captured Task in the observed horizon whose source is absent, **reconcile** emits `capture_source_removed` naming the Task. This is bounded absence, which can also mean the meeting moved outside the horizon; it is not proof of global deletion. The Task, its version and its Log remain intact. Repair changes applied belief only; a later preview may propose recreating a still-active Task's missing source. Operator review should precede that apply.

The ticket mentions `/task/:id/reject`. That existing legacy endpoint records a rejection Log; it does not change canonical Task status. To remove a commitment from the current plan using the existing lifecycle API, the operator can explicitly record `skip` at `/task/:id/action` with schema `ubu.orchestrator.task_action.v1`. Capture does not call either endpoint automatically. Retiring a captured Task suppresses deletion of its source event. Removing `static_window` through the Task edit API returns `capture_static_required`.

## Worked onboarding procedure

1. Stop the local orchestrator and reset its dummy/test database using the existing operator procedure. Start with the required configuration and planning horizon.
2. Pre-populate the dummy Calendar with synthetic timed meetings in that horizon. Use unique palette colours when categories are desired. Keep credentials outside the repository.
3. Configure Google credential/token-cache paths and explicitly enable Google Calendar for this process using the existing desktop-session flow. POST the capture request above with `export_mode: "live"`. Review counts and diagnostics.
4. Generate the plan with `/planning/generate` over the same horizon. Captured commitments are Static Tasks; opaque events occupy capacity and free events do not.
5. GET `/projection/calendar/preview`. The applied set already contains the captured origins, so unchanged captured Tasks have **no operations**. Other independently planned work can have operations.
6. Approve that preview through `/projection/calendar/approve` using the existing authority and live-mode contract. There is nothing to deliver for unchanged captured events. Editing a captured Task's static window and regenerating yields an update of the original meeting.
7. Capture again on demand for new foreign commitments. Reconcile to inspect owned changes and disappearance; completion/reopening and phone-driven edits are deferred.

## Known limits (ticket wording)

1. **Capture only; the calendar is not yet an input device.** Colouring an event does not complete its Task, dragging does not move it, resizing does not change its estimate. That is P1B-33.
2. **On demand.** Nothing polls, so a meeting accepted on the phone is invisible until the operator captures.
3. **No recurrence.** `singleEvents=true` expands a recurring meeting into instances, so each occurrence captures as its own Task with no link between them and no notion of the series.
4. **No attendees, location, description or conferencing data.** A captured Task carries a title, a window, a capacity flag and possibly a category. Everything else on the event is dropped.
5. **All-day events are skipped.** They have no `dateTime`, and UbU plans concrete spans.
6. **A captured Task is Static forever.** Even if the operator would rather UbU moved it, nothing demotes it to Dynamic.
7. **Deleting a captured Task does not delete its event.** The Task leaves the plan; the meeting stays on the calendar, which is almost certainly right for a real appointment but is worth knowing.

