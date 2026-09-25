# Calendar projection preview

P1B-29 supersedes the preview-only persistence behavior described below: previews
now persist projection records and diff against the last applied set. See
[Calendar apply](CALENDAR_APPLY.md) for the current preview/approve contract.

UBU-D0275 describes the switch configuration:

> the dogfooding configuration is one desktop Device with Google Calendar as a projection surface

The desktop can plan the day, but that Plan must eventually be visible on the
phone's Calendar. Google Calendar remains an external projection surface. As
UBU-D0257 states:

> External projection surfaces, including Google Calendar, are not Devices in Phase 1b.

It receives no UbU `device_id` and cannot originate admitted mutations. Consistent
with UBU-D0254 and DESIGN.md §16.8, this preview makes no canonical or external
mutation. A future apply belongs behind the ordinary export and approval checks.

## Local HTTP preview

`GET /projection/calendar/preview` returns schema version
`ubu.orchestrator.calendar_projection_preview.v1`, the current Calendar's
`plan_id` and `stale`, `events`, `operations`, and `diagnostics`.
It reads the same admitted Plan as `GET /calendar/current`; it does not generate
a replacement Plan. Without a current Plan, `plan_id` is null, `stale` is false,
and events and operations are empty.

The preview stores nothing. It compares desired events with an empty existing
set, so every operation is a `create` until P1B-29 persists applied events.
There is no external call or live Calendar client in this implementation.

## Mapping

| Plan or stored input | Desired Calendar event |
|---|---|
| Step title (`summary`) | Event `summary` |
| Step `start_at` / `end_at` | Timed event start and end, in UTC |
| Task `category_tag` through operator-owned `CategoryPalette` | Step `gcal_color_id`, then event `color_id` |
| Task / step `occupies_capacity` | `transparent = !occupies_capacity`; false maps to Google opaque, true to transparent |
| Routine Objective template `reminder_minutes` | Event `reminders_minutes`, preserving order |
| Task id | `external_id`, the validated tail after `task_` |

Every current Calendar step with a valid id becomes an event, including
transparent steps. There is no category, tag or duration filter. Events sort by
`(start_at, external_id)`. The palette's built-in defaults are operator-owned;
for example, personal maps to `3`, work to `9`, and business to `6`. An absent or
unmapped category has no color id.

The operation wire form uses a lowercase `kind`: creates and updates carry an
`event` object; deletes carry `external_id` and `summary`. The pure diff compares
whole events by external id and emits creates, then updates, then deletes, each
sorted by external id. Identical event sets produce no operations. Preview uses
this same diff with an empty existing set.

## Transparency correction

The original synthetic probe showed:

```text
PROBE task  `Work Time` occupies_capacity=false
PROBE step  `Work Time` occupies_capacity=true
```

The old step constructor hardcoded true. Mandatory Dynamic routine occurrences
enter the kernel even when their Task's capacity flag is false, so the assumption
that every kernel step occupied capacity was incorrect. Projecting that value
would mark Work Time busy on the phone instead of transparent, potentially
blocking an entire afternoon on the Calendar.

Kernel-planned steps now use the stored Task's actual flag, defaulting to true
when absent. Non-capacity Static steps still bypass the kernel; mandatory
Dynamic occurrences do not. This fixes the displayed/projection input without
changing the planning-kernel contract or its capacity accounting.

## Reminder source

Quick UbU supplies reminders on routines, not on one-off Tasks. Import stores
those reminders on `routine_instance_template.reminder_minutes`; occurrence
materialization does not lower them onto the Task. Preview reads that template
through `occurrence.routine_objective_id`. A Task's ordinary `objective_id` alone
does not make it a routine occurrence or give it reminders.

Reading the Objective follows the routine lookup pattern already used for
observed durations, avoids an unnecessary Task-schema change and dependency
re-pin chain, and gives one-off Tasks empty reminder lists. Missing or malformed
lists become empty. A valid list consists of nonnegative i64 integers; ordering
and duplicates are preserved. A list containing any invalid member becomes
entirely empty. Reminder changes on the current template are visible even when
the Plan itself has not been regenerated.

## Stable event ids

The supplied Google rule is 5–1024 characters drawn from lowercase `a-v` and
`0-9`. A Task id's lowercase hex tail satisfies that alphabet. `external_id`
strips `task_` and validates the remaining length and alphabet; it does not
generate a random id or require a mapping table. The mapping is reversible by
prepending `task_`. An unmappable step is skipped with
`calendar_event_id_unmappable` naming its Task id.

## Delivery boundary

- **P1B-28:** transparency input, reminder lookup, event model, deterministic diff,
  and read-only preview. All semantics are verifiable without a network.
- **P1B-29:** live client and apply, Google API authorization, deny-by-default
  export gate, and persistence of the last applied projection; likely a separate
  `ubu-gcal-adapter` matching the existing adapter pattern.
- **P1B-30:** reconciliation of external edits, conflicts, and `accept-external`.

The Calendar path is parallel to the existing GitHub preview/approve flow. That
flow's repository, issue, and label request model remains unchanged. Unifying
projection interfaces is deferred until a third surface provides a concrete need.

## Known limits

1. **Preview only.** Nothing is written to any calendar, and nothing records what was written before, so `operations` is always the full create set. The diff is real and tested; it has no persisted "existing" side until P1B-29.
2. **No deletion safety.** Google does not document whether an event id may be reused after the event is deleted. Because ids are derived from Task ids and a routine occurrence gets a fresh Task id each day, this is not reachable today, but a Task deleted and recreated with the same id would be.
3. **One calendar, no calendar selection.** Which Google calendar receives the projection is not modelled; that belongs with the client and its credentials in P1B-29.
4. **Colours come from the operator palette, not from Google.** `CategoryPalette` maps a category to a Google colour id by number. If the user's calendar uses a custom palette, the numbers still apply but the rendered colours may not be what the category name suggests.
5. **All-day events, recurrence and attendees are not modelled.** Every event is a timed, single, attendee-free block. UbU plans concrete spans, and a recurring Google event would fight the daily re-plan rather than help it.
