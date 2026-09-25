# Notice and repair Calendar drift

P1B-29's known limit 2 said:

> **The applied set is UbU's belief, not the calendar's state.** If someone edits
> or deletes an event in Google, UbU will not notice and the next preview will
> propose nothing.

P1B-31 closes that limit on demand. Reconcile reads the Calendar and reports what
changed. Repair is an explicit second step that updates only UbU's applied-set
belief. A subsequent ordinary preview proposes the Google corrections; applying
that preview still requires the existing export gate.

## Ownership and conflict types

**An event is owned by UbU if and only if UbU recorded applying its external ID.**
A Google-looking ID, a matching title, or a known Task does not establish ownership.
Google's IDs and Task-derived IDs share an alphabet, so an ID-shape heuristic
would put real meetings at risk.

| Conflict | Meaning | Repair effect |
|---|---|---|
| `missing` | An owned event is absent from the observed list. | Drop it from UbU's applied set. |
| `drifted` | An owned event's observed fields differ from its applied record. | Replace its applied values with the observed values. |
| `unrecorded` | An observed event is not owned, but its ID derives from an active Task UbU knows about. | Report it; do not adopt or change it. |
| `foreign` | An observed event is neither owned nor linked by ID to an active Task. | Report it; do not adopt or change it. |

Conflicts are sorted by `(conflict_type, external_id)`. Status is `matched` for no
conflicts, `drifted` when any conflict is missing or drifted, and `observed` when
only foreign/unrecorded events are present. Real meetings do not make a calendar
unhealthy just because they were not projected by UbU.

`unrecorded` distinguishes a database-reset case: the Calendar may still contain
UbU-created events after its applied history is lost. If the Tasks are restored
with their IDs, their calendar copies can be identified as likely unrecorded
without claiming ownership. The known-ID set includes every active Task, even
unscheduled Tasks. An inactive/unknown Task provides no such evidence. Existing
applied ownership remains ownership even when its Task is no longer active.

Foreign events are never touched or adopted by reconciliation or repair and are
never added to the applied side of a later diff. Their presence alone generates
no Calendar write. Adopting them as planning constraints is a separate feature;
Quick UbU's `import_from_calendar` is the precedent, not an implicit admission
path here. This follows UBU-D0254's controller trust boundary and UBU-D0257's rule
that Google Calendar is a projection surface, not a Device or mutation authority.

## Repair needs no repair operations

Repair retains exactly the previously owned events still observed, using their
observed values. It creates no external operations and sends no Google writes.
The desired-versus-applied diff already does everything needed afterward:

```text
PROBE[wiped] before repair:
PROBE[wiped] operations: none
PROBE[wiped] missing     `Breakfast` — UbU applied this event and the calendar no longer has it
PROBE[wiped] missing     `Standup` — UbU applied this event and the calendar no longer has it
PROBE[wiped] after repair (applied now holds 0 events):
PROBE[wiped] CREATE Breakfast
PROBE[wiped] CREATE Standup
```

An externally dragged Breakfast is repaired into the applied belief at its dragged
time. The current desired Breakfast then differs from that belief, so the next
preview contains an ordinary update back to the planned time. A foreign Dentist
appointment remains outside the repaired set and produces no operation.

## Endpoints and persistence

`POST /projection/calendar/reconcile`:

```json
{"schema_version":"ubu.orchestrator.calendar_reconciliation.v1","export_mode":"mock"}
```

Use lowercase `live` for the configured Google calendar. Reconcile selects the
same client as approve and calls `CalendarApi::list_events`. It never accepts an
observed-event list in a request. Mock uses the injected stateful recorder when
present; otherwise, like approve, it seeds a recorder from the applied set.
Live requires P1B-30's configured paths and per-process enablement, with unchanged
`calendar_live_export_unconfigured` (503) and `calendar_live_export_not_enabled`
(403) refusals. Setup is documented in [CALENDAR_LIVE.md](CALENDAR_LIVE.md).

The response includes schema_version, reconciliation_id, status, conflicts and
diagnostics. The observation, original applied set, known-ID evidence, conflicts,
and diagnostics are persisted in `projection_reconciliations`, separately from
GitHub label-shaped reconciliation payloads. List failure returns HTTP 502 and
creates no reconciliation. Reconcile does not update the applied set, emit export
boundary Logs, admit Tasks/ExternalEvents, or run the export gate.

`POST /projection/calendar/reconcile/<reconciliation_id>/repair` takes the ID in
the path and requires no body, export mode, credentials or session enablement. It
uses the observation and ownership captured in that reconciliation; it does not
perform another Calendar read. The response includes dropped_events,
updated_events, applied_event_count and remaining_conflicts (foreign/unrecorded).

Repair inserts a Calendar projection result with status `applied`, its repaired
snapshot and empty operation_results. The payload marks `repair: true` and links
the reconciliation ID: `applied` here means the belief was stored, not that any
Google delivery occurred. That result supersedes the old belief even when empty.
The result insert and repaired marker are one transaction. A second repair returns
HTTP 409 `calendar_reconciliation_already_repaired`; it cannot append a duplicate
result. A missing ID returns 404; a GitHub/unsupported record returns 400.

Before the first applied result, reconcile uses an empty ownership set. It records
a null source result in the payload (an empty sentinel in the existing NOT NULL
SQL result_id column) and reserves a typed preview ID for projection bookkeeping,
without creating or claiming a delivered preview. Repair of that observation can
record an empty applied set but cannot acquire ownership of unrecorded events.

Reconcile and repair share Calendar preview/apply's process mutex. Repair trusts
the stored snapshot without a freshness check, as specified: reconcile again if
another apply or external edit happened in between. Existing previews whose
applied base differs after repair are refused by the existing apply conflict
check; obtain a fresh preview.

## Worked example: the operator's wipe-and-copy script

Use a dedicated scratch calendar and its own applied-set database. Verify the
configured destination and credentials as described in the live guide. Never test
a destructive wipe against a relied-on calendar. These are manual operator steps;
automated tests simulate them exclusively with RecordingCalendarApi.

1. Plan and live-apply a synthetic Breakfast and Standup to the scratch calendar.
   Confirm they appear on the phone. Leave the applied record intact.
2. Run the acceptance script that wipes the dummy calendar and copies events over.
   A copied event with a different ID is not a replacement for an owned event just
   because its title is the same. Reconcile classifies exact IDs; unrelated copied
   meetings will be foreign, and known Task IDs lacking ownership will be unrecorded.
3. Enable the process if it was restarted, then observe the configured calendar:

   ```sh
   curl --fail-with-body -sS http://127.0.0.1:7878/projection/calendar/reconcile \
     -H 'Content-Type: application/json' \
     -d '{"schema_version":"ubu.orchestrator.calendar_reconciliation.v1","export_mode":"live"}' \
     > /tmp/ubu-calendar-reconciliation.json
   jq '{reconciliation_id,status,conflicts,diagnostics}' /tmp/ubu-calendar-reconciliation.json
   ```

   For a complete wipe, expect missing Breakfast and Standup and status drifted.
   Applied belief and the old preview remain unchanged until repair. Inspect every
   diagnostic: the existing live wire reader skips entries it cannot represent
   (including all-day/default-reminder events); such a list may be partial. This
   ticket does not broaden the wire model. Do not treat skipped entries as reliable
   deletion evidence without reviewing them.
4. After reviewing the observation, explicitly repair the belief:

   ```sh
   UBU_RECONCILIATION_ID=$(jq -r .reconciliation_id /tmp/ubu-calendar-reconciliation.json)
   curl --fail-with-body -sS -X POST \
     "http://127.0.0.1:7878/projection/calendar/reconcile/$UBU_RECONCILIATION_ID/repair"
   ```

   For the complete wipe, expect dropped_events 2, updated_events 0 and
   applied_event_count 0. Foreign/unrecorded conflicts remain in remaining_conflicts.
   Nothing has been sent to Google by repair.
5. Request and review a new preview, then apply it through the normal gate:

   ```sh
   curl --fail-with-body -sS http://127.0.0.1:7878/projection/calendar/preview \
     > /tmp/ubu-calendar-repair-preview.json
   jq '{events,operations,diagnostics}' /tmp/ubu-calendar-repair-preview.json
   ```

   Expect two creates after a complete wipe, or one update after dragging only
   Breakfast. No foreign ID should appear in an operation. Only after reviewing:

   ```sh
   jq '{schema_version:"ubu.orchestrator.calendar_projection_approval.v1",
        preview_id,authority_source:"automation_worker",export_mode:"live"}' \
     /tmp/ubu-calendar-repair-preview.json > /tmp/ubu-calendar-repair-approval.json
   curl --fail-with-body -sS http://127.0.0.1:7878/projection/calendar/approve \
     -H 'Content-Type: application/json' --data-binary @/tmp/ubu-calendar-repair-approval.json
   ```

   Verify status applied and each operation's outcome, then confirm the phone's
   events and untouched foreign meetings. Reconcile again: owned events should
   match; foreign/unrecorded-only conflicts yield observed rather than drifted.

Neither reconcile nor repair invokes Legitimizer::gate_export_projection: the
first reads externally, and the second changes only local projection bookkeeping.
The later apply still gates every external write. Per-session enablement is a
network opt-in, not a replacement for endpoint authentication or the export gate.

## Known limits

1. **Reconciliation is on demand.** Nothing polls. UbU notices a wiped or edited calendar only when the operator asks, so the phone can disagree with the desktop until then.
2. **Drift detection is whole-event.** Any difference between the observed and applied event is one `drifted` conflict; the response does not say which field moved.
3. **Foreign events are not planning constraints.** A real meeting on the calendar is reported and ignored. The planner still does not know the operator is busy then.
4. **Repair trusts the observation completely.** If a read returns a partial or stale list, repair will drop events that do exist, and the next apply will recreate them. Recreation is safe because the ids are derived, but the calendar will churn.
5. **No history.** Each reconciliation is stored, but there is no view of how a calendar drifted over time.

