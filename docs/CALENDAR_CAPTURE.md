# Calendar capture

## A colour decides the placement

From P1B-55, and this section overrides anything below that says otherwise:

- **An event with no colour is work for UbU to schedule.** It is captured as a
  Dynamic Task. Its duration is the event's length, as
  `duration_estimate: {"type":"fixed","seconds": end - start}`, and nothing
  else of its time is kept: no `static_window`, no `allowed_time_range`. The
  planner decides when. It always occupies capacity, whatever the event's
  Busy or Free setting, and it has no category.
- **An event with any colour is a commitment at its own time.** It is captured
  as a Static Task, exactly as every event was before: `static_window` from
  the event, `category_tag` from the palette's inverse, and
  `occupies_capacity` from the event's transparency. A colour that maps to no
  category, or to several, is still a colour. The event is Static and its
  category is unknown.

This is the inverse of export. A Static Task is exported in its category's
colour and a Dynamic Task with none, so an event UbU exported for a Dynamic
Task and captured again is Dynamic again. Busy against Free decides nothing
about placement. That was a Quick UbU workaround and is retired.

What follows from it:

- **Two uncoloured events at overlapping times do not collide.** Neither is
  Static. They are two pieces of work, placed one after the other.
- **The first preview after a capture moves them.** A Dynamic capture is in the
  applied record at the time its event had. The Plan puts it somewhere else,
  so the preview proposes an `update` of the event to its new window, with no
  colour. A Dynamic capture that does not fit is left where it is: a captured
  event is never deleted.
- **A captured Task follows its event's colour.** A captured event is in the
  applied record, so later captures do not treat it as foreign. But when it
  **gains or loses** its colour, the rule is applied again to the same Task.
  Losing the colour removes `static_window` and `category_tag` and sets the
  duration from the event's length. Gaining one removes `duration_estimate`
  and sets `static_window` to the event's window as it then stands, with the
  colour's category. The Task keeps its id; its version goes up by one. A
  change from one colour to another is not read: it is `capture_owned_drift`,
  as before.
- **This reads captured Tasks only.** An event UbU exported for a Task of its
  own is never offered to this rule. Its colour is a gesture: on a Dynamic
  Task it means done. See [CALENDAR_INTERACTION.md](CALENDAR_INTERACTION.md).
  So colouring a to-do that came from the calendar pins it; colouring one
  that was made in UbU completes it.
- **An event UbU cannot own is Static whatever its colour.** An instance of a
  recurring event cannot be written to, so it cannot be moved. Uncoloured, it
  still stays a commitment at its own time, and its `capture_colour_absent`
  says so.
- **Nothing shorter than one planning second is captured.** A duration of
  zero is invalid, and a duration is never invented. From the Calendar API
  such an event is skipped at the wire as `calendar_event_skipped`; an event
  that reaches capture with no span is refused as `capture_event_invalid`.
- **An all-day event is skipped**, as before, with
  `capture_all_day_unsupported`. It has a date and no time, so it carries no
  duration and cannot be scheduled.
- **UbU's own stale exports are not captured**, from P1B-57. An event UbU
  stamped when it created it, arriving as foreign, is UbU's echo from a store
  it no longer has. It becomes no Task, counts as skipped, and is reported
  once as `capture_stale_export`. See
  [CAPTURE_PROVENANCE.md](CAPTURE_PROVENANCE.md).
- **Nothing converts Tasks already captured.** A store captured before P1B-55
  holds every event as Static. Capture into a fresh store.

| Diagnostic | Sentence |
|---|---|
| `capture_colour_absent` | Calendar event `<id>` has no colour, so it is taken as work for UbU to schedule: a Dynamic Task of the event's length, at no fixed time |
| `capture_colour_absent`, for an event UbU cannot own | Calendar event `<id>` has no colour, but UbU cannot own it and so cannot move it: it stays a commitment at its own time, with no category |
| `capture_colour_unmapped` | unchanged: Static, no category; map the colour in Settings |
| `capture_colour_ambiguous` | unchanged: Static, no category |
| `capture_all_day_unsupported` | list event `<id>` entry N: all-day event has no dateTime; it carries no duration, so it cannot be scheduled and is skipped |
| `capture_stale_export`, from P1B-57 | Calendar event `<id>` was created by UbU for a Task this store does not have, so it is left alone and becomes no Task |

`capture_colour_absent` is not a deficiency. It is the ordinary case for a
to-do, and it says what was done with the event.

## History

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
| Event `description`, trimmed and at most 16,384 bytes | `description`, only if the Task has no non-blank description | Never exported; insert and PATCH omit it |
| `start.dateTime`, `end.dateTime` | `static_window.start`, `.end` | Scheduled static start/end as UTC RFC3339 |
| `transparency: "transparent"` | `occupies_capacity: false` | `transparency: "transparent"` |
| `transparency: "opaque"` or absent | `occupies_capacity: true` | `transparency: "opaque"` |
| Unique palette `colorId` | Matching `category_tag` | The same palette colour |
| Shared palette `colorId` | No category; `capture_colour_ambiguous` | Preserve the recorded source colour |
| Unmapped `colorId` | No category; `capture_colour_unmapped` | Preserve the recorded source colour |
| Absent `colorId` | **A Dynamic Task**, from P1B-55: `duration_estimate` and no `static_window`; `capture_colour_absent` | No colour, at the window the Plan chose |
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

Pure `plan_capture` leaves `task_id` as `None` for new sources; random IDs are allocated only at admission. New payloads have user provenance, source kind `google_calendar`, a static window and capacity flag. A resolved category is also included in `tags`, as core validation requires. They use the ordinary `user-capture` compartment. Existing user metadata outside the captured fields is retained on a source update. Task admissions and the final applied snapshot are separate store writes; retry after interruption recovers the Task through provenance rather than duplicating it.

All-day entries produce `capture_all_day_unsupported`. Invalid IDs are skipped rather than converted to derived IDs. Expanded recurrence instances still have to satisfy this ID contract.

## Bounded observation and disappearance

`CalendarApi::list_events` takes a `CalendarTimeRange`. Google requests include percent-encoded `timeMin` and `timeMax`, with `singleEvents=true` on every page. The recorder applies the same exclusive overlap rule: event start is before the range end and event end is after the range start. Bounds use the planner's existing horizon resolution: latest stored Calendar window if present, otherwise the process planning clock and configured horizon.

Reconciliation compares only applied entries overlapping the requested range. Its stored observation carries the other applied entries separately, and repair preserves them. A bounded response cannot establish that an out-of-range event was deleted.

For an active captured Task in the observed horizon whose source is absent, **reconcile** emits `capture_source_removed` naming the Task. This is bounded absence, which can also mean the meeting moved outside the horizon; it is not proof of global deletion. The Task, its version and its Log remain intact. Repair changes applied belief only; a later preview may propose recreating a still-active Task's missing source. Operator review should precede that apply.

The ticket mentions `/task/:id/reject`. That existing legacy endpoint records a rejection Log; it does not change canonical Task status. The `skip` action is restricted to routine occurrences and cannot retire a captured one-off Task. The existing `complete` action at `/task/:id/action` changes lifecycle state when the work actually is complete; there is no generic canonical reject transition exposed for a captured Task in this ticket. Capture does not call these endpoints automatically. A captured Task leaving the plan suppresses deletion of its source event. Removing `static_window` through the Task edit API returns `capture_static_required`.

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
4. ~~No attendees, location, description or conferencing data.~~ Revised 2026-10-04, P1B-63: **No attendees, location or conferencing data.** Capture may also take the event's own `description`, only when the Task's existing description is absent or whitespace-only. A non-blank description, including Clarify's accumulated answers or text edited on Tasks, is never overwritten. Later changes to Google's notes do not update a Task that already has notes; read and edit its notes on Tasks. Leading and trailing whitespace is trimmed, with interior text and markup preserved literally. The shared `clarify::MAX_DESCRIPTION_BYTES` bound is 16,384 UTF-8 bytes after trimming. Longer notes are refused whole, never truncated: `capture_description_too_large` names only the event id and bound, while the Task still captures with its title and does not count as skipped. Missing or blank notes add no description key to a new Task. A non-string wire description is invalid. Notes never go back to Google: insert and PATCH bodies omit description, so a PATCH preserves the operator's calendar notes. Notes alone do not cause projection drift or an updated capture count.
5. **All-day events are skipped.** They have no `dateTime`, and UbU plans concrete spans.
6. ~~A captured Task is Static forever.~~ No longer true, from P1B-55: an uncoloured event is captured as Dynamic, and removing an event's colour demotes its Task to Dynamic at the next capture.
7. **Deleting a captured Task does not delete its event.** The Task leaves the plan; the meeting stays on the calendar, which is almost certainly right for a real appointment but is worth knowing.
