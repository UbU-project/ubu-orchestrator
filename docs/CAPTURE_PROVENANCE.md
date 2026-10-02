# Capture provenance: UbU knows its own events

**This file is contract.** A ticket that changes the stamp, who writes it, how
it is read or what capture does with it changes this file in the same commit.

## The stamp

When UbU creates a calendar event it writes one private extended property on
it, naming the Task it minted the event for:

```json
"extendedProperties": {"private": {"ubu_task": "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e70"}}
```

The key is `calendar_wire::UBU_TASK_PROPERTY`. The value is the Task id.

**Only an insert writes it.** `event_request` adds it in the same arm that adds
`id`, and nowhere else. A PATCH never stamps. UbU also patches events it did
not create: the operator's own events, which UbU captured and now manages, and
which keep his id. Stamping a patch would mark his events as UbU's, and a new
store would then discard his commitments. A PATCH leaves an omitted field
alone, so a stamp written at insert survives every later patch. An insert that
Google answers with 409 is converted to a patch, and that patch carries no
stamp either.

`event_body` and the PATCH body are unchanged, and still match Quick UbU's
writable body byte for byte.

## Reading it back

`calendar_wire::ubu_created_ids` walks a Google list and keeps each event id
whose stamp equals `task_` plus that id. **The round trip is the predicate, not
mere presence.** An id UbU mints is its Task id without the prefix, so a stamp
that names the Task the id is the handle of can only have been written by that
insert. A stamp naming some other Task is evidence of nothing: the event is
captured as any other. That is what an event UbU re-creates for a captured Task
carries, since it keeps the operator's id. Every absence, of the list, the
property or a string value, yields fewer ids and never a failure.

The ids travel beside the events, not inside them. `DesiredEvent` has no field
for the stamp, so the applied record, the diff and reconciliation compare
exactly what they compared before. `CalendarApi::take_ubu_created_ids` returns
them, drained after a list as diagnostics are and accumulated across pages.
Its default is none, which reads as "nothing is known to be UbU's".

## What capture does with it

Only a **foreign** event reaches `plan_capture`: one the store has no applied
record for, and whose id is not the handle of an active Task. A stamped event
arriving there is UbU's own echo from a store UbU no longer has. It becomes no
Task. It is not adopted, not written to and not deleted. It counts in
`skipped`, and the capture reports it once:

```text
Calendar event `<id>` was created by UbU for a Task this store does not have, so it is left alone and becomes no Task
3 Calendar events were created by UbU for Tasks this store does not have, so each is left alone and becomes no Task: `<id>`, `<id>`, `<id>`
```

The code is `capture_stale_export`. With more than three it names the first
three and counts the rest. Ids only: a title is never echoed.

What it does not change:

- **A store that knows the Task.** The event is `unrecorded`, not foreign, and
  `capture_unrecorded_event` reports it as before.
- **A store that applied the event.** It is owned, and unchanged.
- **An event this store holds a captured Task for.** It is that Task's event
  whatever it carries.
- **Reconciliation.** It does not read the stamp. A stale export is still
  listed there as `foreign`, under the sentence "this event was not created by
  UbU and will not be touched". For a stamped event the first half of that
  sentence is wrong. It is still not touched.

## What the stamp cannot do

**An event UbU created before P1B-57 carries no stamp and cannot be
recognised.** UbU mints an event id from a Task id's tail, in the alphabet
`a` to `v` and `0` to `9`, which is also the alphabet and roughly the length of
an id Google mints. Without a stamp, a leftover of UbU's and an event of the
operator's cannot be told apart, and the leftover is captured as his.

So the live rehearsal's calendar reset is still the instruction. From the
first approve after P1B-57, what UbU creates is stamped, and a later run
reports it and does not capture it.

A copy of the calendar made by another tool may or may not carry private
extended properties across. If it does not, a copied leftover is unstamped,
and is captured.

## Where it is asserted

`tests/calendar_wire.rs`: the insert body, that no PATCH body carries the
property, the reader's tolerance, and an insert read back through the reader.
`tests/capture_provenance.rs`: the three kinds of event, the diagnostic, both
stores of a rehearsal over HTTP, and the store that knows the Task.
`tests/calendar_mock_seed.rs`: a stamp in the mock calendar fixture.
