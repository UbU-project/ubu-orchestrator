# Calendar occupancy: what UbU owns, and what it only knows is there

P1B-51. This replaces the capture half of P1B-44's "tolerate at parse, refuse
at capture". Reconciliation is unchanged.

## The rule

**UbU owns an event whose id can round-trip through `external_id`. Any other
event it captures is occupied time it will never write to.**

`calendar_projection::external_id` is the ownership rule. An id is ownable
when it is 5 to 1024 characters of lowercase base32hex, `a` to `v` and `0` to
`9`, because that is the alphabet Google accepts for a client-supplied event
id and so the only kind of id UbU could ever have minted. An instance of a
recurring event has the shape `{base32hex}_{timestamp}`, for example the
invented `abc123def456ghij_20260928T163000Z`. The underscore and the
uppercase letters are outside the alphabet, so UbU cannot own it.

Before P1B-51 capture refused such an event with `capture_event_not_ownable`
and admitted nothing. The rule was right about ownership and wrong about the
consequence: refusing to own an event is no reason to treat its time as free.
The planner placed work over commitments that were really there, and the
operator covered each one by hand with a Busy placeholder.

## What capture does now

For an event it cannot own, capture:

- admits one Static Task at the event's window, with the category the event's
  colour maps to, exactly as for any other foreign event;
- gives it a **minted** handle. The Task id is never derived from the event id;
- keeps the event id in `provenance.source.source_id`, which is the dedupe
  key, so a repeat capture finds the same Task and updates it in place;
- reports `capture_occupancy_only`, naming the event id and never its title.
  A recurring commitment's title is among the most private text in the
  calendar;
- **does not add the event to the applied record.** Capture writes no
  `projection_results` row for it.

The last point is what keeps ownership honest. The applied record is UbU's
statement of what it applied to the calendar. An event in it is compared for
drift, can be reported `missing`, and is what repair rewrites. An event UbU
cannot own is none of those things, so it is read afresh by every capture
and stays `foreign` to every reconcile.

| | an ownable foreign event | an event UbU cannot own |
|---|---|---|
| becomes a Static Task | yes | yes |
| Task handle | minted | minted |
| event id in `provenance.source` | yes | yes |
| occupies planning capacity | unless the event is Free | unless the event is Free |
| entered in the applied record | yes, capture claims it | **no** |
| in the desired export set | yes, under its own id | **never** |
| reconcile after capture | matched | still `foreign`, still `capture_event_not_ownable` |
| moved in the calendar | the Task follows | the Task follows, on the next capture |
| capture diagnostic | colour diagnostics only | `capture_occupancy_only` |

## Why it can never be written back

Three pieces were already in place, and capture only had to stop refusing
before it reached them:

1. `calendar_capture::capture` mints the handle with
   `UbuId::new(ObjectType::Task)` when the source is new.
2. `existing_by_source` maps a Google id to the Task captured from it.
3. `calendar_projection::external_id_for` returns `None` for a captured
   origin that cannot round-trip, and never falls back to the Task's own id,
   so `desired_events` drops the step. `captured_origins_override_derived_ids_and_never_fall_back`
   asserts that. With no desired event and no applied event there is nothing
   for `diff` to create, update or delete.

The preview reports the dropped step as `calendar_event_id_unmappable`. That
diagnostic is older than this change and reads as a fault; for an occupancy
Task it is the exclusion working.

## What is asserted

`tests/calendar_occupancy.rs`:

- a recurring instance and an ownable event are both captured, and only the
  first carries `capture_occupancy_only`;
- both are Static Tasks with the colour's category, and the unowned one's
  handle is minted with its Google id in `provenance.source`;
- the preview excludes the unowned Task and still includes the ownable one;
- a repeat capture admits no object and writes no envelope;
- **approving after an unowned capture leaves the calendar byte for byte
  unchanged**, and when the approve does write UbU's own work, no call of any
  kind names the unowned event;
- reconcile reports exactly one conflict for it, `foreign`, before and after;
- **the planner places no Dynamic work over its window**, against a control
  in which the same backlog does run through that window;
- an unowned event moved in the calendar moves the occupied window in place.

`tests/calendar_foreign_tolerance.rs` keeps the P1B-44 assertions that still
hold: the calendar is only ever read, no title appears in a diagnostic, no
`projection_results` row comes from capture, both reconciles report the same
foreign conflict, and approving leaves the recorded events unchanged.

## What is unchanged

- Reconciliation. `not_ownable_diagnostic` still serves it, with the same
  words, and the Calendar screen still groups those conflicts apart.
- An all-day recurring instance. It has no concrete `dateTime`, fails at the
  wire, and keeps `capture_all_day_unsupported`.
- An event with an empty title or an unusable window keeps
  `capture_event_invalid`, whether or not UbU could own it.
- Recurrence itself is not imported. Each instance inside the planning
  horizon is one Static Task; there is no series, and an instance beyond the
  horizon is not seen until the horizon reaches it.
