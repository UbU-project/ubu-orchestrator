# Bootstrap from the calendar

The calendar is the seed. Quick UbU data is **not being imported**. The existing
`POST /import/quick-ubu` remains available over HTTP and is unchanged, but it is
not part of this bootstrap and has no UI.

Capture admits **only foreign events** as new Tasks. In a bootstrap, those are
events on a calendar UbU has not yet applied to: ownership comes from UbU's
applied record, not an event's title or ID shape. Known-but-unrecorded events are
not foreign. Capture remains manual and uses the existing admission path; this
ticket introduces no new import machinery or background polling.

## Check the inverse palette first

In Setup, read the Colours card's **colour-to-category** view before capture.
Compare it with the colours in the intended calendar. Set category colours until
each colour you intend to import has the intended unique category. The forward
view shows each effective colour and whether it came from a Setting, file or
default. A Setting overrides the file and built-in default immediately; reverting
it exposes the fallback.

A colour not mapped by the palette produces a Task **with no category**. Two
categories using the same colour are a collision and also produce no category.
Do not assume the eleven built-in defaults describe the operator's calendar.

Capture reports three distinct advisory diagnostics, naming the source event:

| Diagnostic | Meaning | What to correct |
|---|---|---|
| `capture_colour_ambiguous` | Multiple categories map to the event colour. | Resolve the collision in the inverse view before bootstrap capture. |
| `capture_colour_unmapped` | No category maps to the event colour; the diagnostic names the colour ID too. | Map that colour in Settings before bootstrap capture. |
| `capture_colour_absent` | The event has no explicit colour. | Give the source event a suitable colour before capture, or assign the Task category explicitly afterwards. |

All three are advisory: the Task is still admitted without a category. Mapping a
colour after capture does **not** automatically recategorise already-owned Tasks;
ordinary foreign capture has already made those sources owned. This is why the
palette review comes first. Capture otherwise retains its existing foreign
classification, transparency-to-capacity conversion and concrete event window.

## Bound the observation

`calendar_reconciliation_service.rs` states:

> A bounded observation says nothing about commitments outside its range.

Reconciliation and foreign capture use `CalendarTimeRange::planning`. For a fresh
workspace without a stored Calendar window, it spans the planning clock's now
through `now + UBU_PLANNING_HORIZON_SECONDS`, which is one week unless set. Widen
`UBU_PLANNING_HORIZON_SECONDS` for the bootstrap run, start the orchestrator with
that configuration, then restore the usual value and restart afterwards. Nothing
automates widening or restoration. Widening the forward span does not make past
years part of the bootstrap.

**Existing range precedence matters:** `planning_service::resolve_time_window`
uses a valid latest stored Calendar window before the configured horizon fallback.
In a workspace with such a window, changing only the environment variable does
not widen observation. The stored window would also need to cover the intended run. This ticket does
not change that stored window, change the resolver, or provide a horizon editor. The environment-only
procedure above applies to the fresh-workspace fallback, not every possible
existing workspace.

Capture's existing recent-owned-event lookback supports completion/reopening; it
does not widen foreign bootstrap capture. An event must overlap the selected
planning horizon to be considered as a new foreign commitment.

## Operator procedure and acceptance

No agent test has contacted Google or exercised this against an account. Use the
operator's dummy account for acceptance, after completing P1B-41's six steps.

**Resolved in P1B-43:** the blocker described in this paragraph no longer
exists. The Calendar export is now gated as the automation worker whoever
approves; see [Calendar apply authority](CALENDAR_APPLY_AUTHORITY.md). The
paragraph is kept as the record of what P1B-42 found.

There was a **pre-existing P1B-41 approval blocker**: `ubu-ui`'s `approveCalendar`
sends `authority_source: "user"`, but the backend export gate requires
`automation_worker`. It reports `calendar_export_rejected` for user authority.
P1B-42 explicitly leaves the Calendar screen and approval wrapper unchanged;
resolve that separate issue before expecting P1B-41's approval acceptance to pass.
P1B-42's offline tests use the backend's accepted mock-export authority. No
credential or account was used to establish this finding.

Then follow this order:

1. In Setup, confirm the eleven default category rows and inspect the inverse.
   If Settings/file overrides already exist, their actual origins appear instead.
2. Compare the inverse against the calendar's colours. Map unmapped colours and
   resolve collisions **before** the first capture.
3. Change one colour and confirm its origin becomes `setting`.
4. Take a new Calendar preview and verify a Static Task in that category has the
   new colour without restarting. An older preview retains its reviewed payload.
5. Enable the configured Google session as needed, choose the bootstrap horizon
   as above, and capture events UbU did not create. Confirm the resulting Task
   categories match the inverse.
6. Confirm any event with an unmapped or absent colour produces the diagnostic
   naming it. Restore the normal horizon configuration after the run.

Captured Tasks are **Static and pinned to their event windows**. Transparent
source events do not occupy capacity; opaque ones do. Nothing infers that a
calendar commitment was meant to be Dynamic work.

## What a calendar cannot carry

Recurrence is **not imported**. From P1B-51 each recurring instance inside the
planning horizon is captured as occupied time, a Static Task UbU does not own,
as detailed below and in [Calendar occupancy](CALENDAR_OCCUPANCY.md). There is
still no series: reauthor a routine natively using the Routines screen or the
existing Objective/routine endpoints. This does not invent a recurrence importer.

Preferences, dependencies, bounded `after` relations, and `establishes`/`requires`
also do not survive: none exists on the captured calendar event. Reauthor these
through the native mechanisms. Importing calendar commitments cannot reconstruct
the semantics of the abandoned Quick UbU snapshot.

The palette review cannot recover those relationships. It determines categories
only, with the diagnostic and ownership limits above.

## Recurring instances: observe, name, record the occupancy

Google's [event resource](https://developers.google.com/workspace/calendar/api/v3/reference/events)
requires client-supplied IDs to use lowercase base32hex (`a`–`v`, `0`–`9`),
5–1024 characters. Expanded recurring instances have a different shape:
`{recurringEventId}_{originalStartTime}`, for example the invented
`abc123def456ghij_20260928T163000Z`. Google's
[recurring-events example](https://gsuite-developers.googleblog.com/2011/12/calendar-v3-best-practices-recurring.html)
illustrates the underscore and UTC timestamp suffix. UbU treats observed IDs as
opaque handles; it does not depend on this example being an exhaustive grammar.

Before P1B-44, observation incorrectly applied the client-supplied ID rule to
returned event IDs. It dropped recurring instances before reading their windows,
colours or summaries. Observation now sees every event its existing timed-event
model can represent; ownership decides what UbU may claim. Missing required
fields, invalid timestamps, cancelled items and unsupported all-day events still
follow their existing refusals. Listing diagnostics identify the entry index and
readable event ID, never its title or other item content.

A recurring instance is observed as `foreign`, and UbU cannot own it. The reason
in `calendar_projection.rs` remains:

> Captured ids must validate on their own; deriving a fallback would duplicate the meeting.

Until P1B-51 capture refused it outright and admitted no Task, so its time looked
free to the planner. From P1B-51 capture records the occupancy and claims nothing:
it admits one Static Task under a minted handle, keeps the event ID in
`provenance.source` as the dedupe key, reports `capture_occupancy_only`, and
writes no ownership record. Reconciliation after capture still sees the event as
foreign, with `capture_event_not_ownable`. The Task is never in a desired export
set, so no preview or approval can write to the event. Neither foreign group is
repairable. Importing recurrence as a series is later work and needs its own design.

| Code | Meaning | Operator action |
| --- | --- | --- |
| `capture_occupancy_only` | Capture: the named event ID cannot be a UbU Task handle, so its time was recorded as an occupied window UbU will never write to. | None. Keep managing the commitment in Calendar; the next capture follows it if it moves. |
| `capture_event_not_ownable` | Reconcile: the named event ID cannot be a UbU Task handle. The event is foreign and stays foreign. | None. Do not substitute an ID to force ownership. |
| `capture_event_invalid` | An empty title or unusable concrete window prevents Task admission. | Correct the title or timed window at its source, then retry capture. |
| `capture_all_day_unsupported` | An all-day item has no supported concrete `dateTime` window. | Keep it outside capture, or explicitly change it to a timed commitment if that reflects its actual meaning. |

Malformed wire items instead produce `calendar_event_skipped`. Use its index and
ID to locate and correct the source where appropriate; cancelled items are
intentionally not imported.

**An uncaptured commitment occupies no capacity. The planner may place work over
it.** That is now true only of what capture cannot represent at all: all-day
items, malformed items, and events with no title or no usable window. A recurring
instance is captured and does occupy capacity, unless the event is Free:
`plan_capture` sets `occupies_capacity: !event.transparent`.

The Calendar reconcile view still separates `foreign` from `foreign, cannot be
captured` and warns that UbU cannot see the latter when planning. That wording
describes reconciliation before a capture. After a capture those instances are
occupied windows in the Plan, and the group means only that UbU does not own them.

**The P1B-44 workaround is removed.** A Busy placeholder placed by hand over a
recurring commitment is no longer needed and, once the instance is captured,
doubles the occupancy for that span. Delete the placeholders before capturing.

For operator acceptance, add a recurring series in the dummy account and run
**reconcile before capture**. Check the group, its reason and horizon
count. Capture must record those instances as occupied time, with
`capture_occupancy_only`, while continuing to capture ordinary timed events.
Reconcile again: the instances must remain foreign without duplicates. Also create an ordinary event by hand and reconcile before capturing
it: it belongs in plain foreign, without a repair control. Capture claims ordinary
foreign events immediately, so reversing this order hides that classification.
