# Occurrence overrides

UBU-D0289 already names the action this implements:

> It reports the occurrences it cannot place and asks the user to triage: skip an occurrence, move it through an occurrence override, or change other commitments.

P1B-34 rejected a routine drag with “moving or resizing it needs an occurrence override (UBU-D0289)”. A materialized occurrence is rebuilt from its routine, so a Task-only window edit would disappear on the next generate. The canonical recurrence schedule now records `overrides: [{local_date, start, end}]` beside `exdates`. Each date may have one positive timestamp span.

## Identity and placement

The occurrence key is unchanged by setting or clearing an override. Neither `schedule_version` nor `template_version` is incremented by this operation. The prototype illustrates why:

```text
PROBE[before] Shower and dress   07:00..12:00  key=t1/planned/2026-09-27T07:00:00
PROBE[after]  Shower and dress   15:00..15:30  key=t1/planned/2026-09-27T07:00:00
PROBE[keys]   unchanged across the override = true
PROBE[again]  Shower and dress   15:00..15:30  key=t1/planned/2026-09-27T07:00:00
```

A placement decision for one day neither supersedes the Task nor creates object churn on later generates. The key still records the original placement, including `planned`, while the overridden Task carries only a `static_window`, with no `allowed_time_range`. The planner honours that exact span even when its length differs from the declared duration.

The decision is keyed by the routine's local date rather than its occurrence key. A later title or nominal-start template edit can change the template version and hence occurrence identity, but the dated override remains and is applied to the new instantiation. Overrides do not enter the key computation.

## Phone and triage

Dragging or resizing a projected routine occurrence writes the date override through ordinary Objective admission with user authority, refreshes any active cached occurrence window, and appends an Objective Log with the established `google_calendar` source marker. The next preview accepts the observed window without proposing a compensating operation. A previous start fact does not prevent this explicit placement decision. General Task edits still reject occurrences.

| Phone gesture | Result |
| --- | --- |
| Drag a Dynamic one-off Task | Position is ignored. |
| Drag a Dynamic routine occurrence | That local date is pinned with an override. |
| Drag a Static routine occurrence | That local date receives the new override window. |

A routine has a durable schedule home for the dated decision. This deliberately refines P1B-34's Dynamic-position rule. When one captured observation both moves and colours an initially Dynamic occurrence, the completion interpretation uses that initial partition and records the dragged window. Already completed Tasks retain P1B-34's ignored-drag behaviour.

Triage uses the same validation and admission service without Calendar transport:

```http
PUT /routine/obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01/override/2026-09-27
Content-Type: application/json

{"schema_version":"ubu.orchestrator.routine_override.v1","start":"2026-09-27T15:00:00Z","end":"2026-09-27T15:30:00Z"}
```

`DELETE` on the same path takes no body and removes the entry. Clearing restores the derived nominal window; setting a nominal-looking override would instead leave a pin that survives template edits. Both responses report `schema_version`, `objective_id`, `local_date`, `overridden`, and `diagnostics`. Triage Logs carry no Calendar source marker. An occurrence need not already be materialized; a later generate will apply the stored decision.

## Guards and diagnostics

The routine must occur on the named date under its timezone, rule, enabled bounds, and exdates. Both endpoints reject a non-occurrence date before writing. A stored override imported through another canonical path is diagnosed by the instantiation date loop if the routine no longer occurs on that date.

| Diagnostic | Meaning |
| --- | --- |
| `routine_override_outside_allowed_range` | The explicit span is outside the template's declared local range. It is honoured. |
| `routine_override_violates_after_bounds` | Its start breaks a matched predecessor's minimum or maximum bound. It is honoured. |
| `routine_override_no_occurrence` | The schedule does not produce an occurrence on that date. Endpoint admission is rejected; instantiation reports the unused entry. |

Contradicting declarations and derived bounds are reported, never used to clamp an explicit override. Instantiation diagnostics are emitted only for dates in its reporting window.

Section 8's date-relative wording is interpreted literally: a span must be positive (including at materialization's whole-second resolution) and stay between the named local day's midnight minus one configured planning horizon and the next local midnight plus one horizon. This permits future-date triage independently of today's horizon. The existing one-off move guards remain unchanged. Invalid or unresolvable local dates are rejected. DELETE checks the nominal window it would restore against these same guards. A malformed old pin can therefore be removed when its nominal restoration is valid. Guard failures make no canonical writes.

## Known limits

1. **One override per date per routine.** A routine that occurs twice on one date cannot have its two occurrences overridden differently. The Phase 1b schedule subset has no such rule, so this is not reachable today.
2. **An override is a window, not a duration change.** It says when, not how long, and a span that disagrees with the routine's `duration_estimate` is honoured as given.
3. **Overrides are never garbage-collected.** An override for a past date stays on the schedule. Nothing reads it again, but it accumulates.
4. **A template edit does not revisit overrides.** Changing `nominal_start` leaves existing overrides in place, which is judgment call 3 working as intended and may still surprise.
5. **Gesture ordering across polls can diverge.** Google returns whole event state with one `updated` timestamp and no per-field history, so UbU cannot tell a drag-then-colour from a colour-then-drag within one observation. Across two captures it can: colouring, capturing, then dragging leaves the drag applying to a completed Task, which P1B-34 ignores. The same two actions before a single capture record the dragged window. This is not introduced here and is not fixed here.
6. **Overrides do not participate in `UBU-D0289` triage automatically.** The endpoint exists; nothing proposes an override when a day cannot hold its mandatory work.

