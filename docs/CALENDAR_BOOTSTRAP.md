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
through `now + UBU_PLANNING_HORIZON_SECONDS`. Widen
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

Recurrence does not survive this capture path. A weekly routine represented by
N observed occurrences becomes **N one-off Static Tasks**, not a Routine. Reauthor
routines natively using P1B-38's Objective/routine endpoints; there is still no
Routines screen. This does not invent a new recurrence importer.

Preferences, dependencies, bounded `after` relations, and `establishes`/`requires`
also do not survive: none exists on the captured calendar event. Reauthor these
through the native mechanisms. Importing calendar commitments cannot reconstruct
the semantics of the abandoned Quick UbU snapshot.

The palette review cannot recover those relationships. It determines categories
only, with the diagnostic and ownership limits above.
