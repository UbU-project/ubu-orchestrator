# Static containment

For two capacity-occupying Static Tasks without routine-occurrence exceptions:

| Window relationship | Result |
| --- | --- |
| Proper containment: one window encloses the other, with at least one different edge | Share committed time; keep both Tasks on the Calendar |
| Equal start and end | `static_task_collision`, a warning from P1B-54: both keep their windows, the span is busy, and the Plan is made |
| Partial overlap: neither window contains the other | The same `static_task_collision` warning |

Sharing just the start or just the end qualifies as proper containment. Windows
that only touch do not overlap. Equal spans have no “during” relationship: two
Tasks pinned to exactly the same five minutes remain a double-booking to resolve.
Existing mandatory routine overlap handling is unchanged.

Containment reuses `committed_clusters`. Its union-find joins connected fixed
placements. A **carrier** is the one Task sent to the kernel to reserve the
group's union window. It is chosen by earliest start, then latest end, then id.
External dependencies are inherited by the carrier; internal edges are removed
from the kernel request. A containing Static prerequisite gets the same
containment exception in conflict detection.

The whole union span stays busy, including the space between small contained
chores. Dynamic work must be placed outside it. This is the conservative reading
of a pinned block: pinning a six-hour commitment does not invite the planner to
fill its gaps with backlog.

The carrier is restored to its own true window in the response, and covered
members are reattached at their own true windows. Every capacity member remains
on the Calendar with `occupies_capacity: true`; capacity is reserved once at
planning time. Non-capacity Tasks still bypass clustering and retain their
existing direct Calendar projection.

Each group with no routine occurrence emits one plain diagnostic,
`static_tasks_share_committed_time`, for each Task that has others inside it
and is inside none itself, counting the Tasks inside it. In a group formed by
containment alone that Task is the carrier, and the count is every other
member:

```text
1 Static Task happens during `<carrier id>`; the whole span is busy and every one of them stays on the Calendar
2 Static Tasks happen during `<carrier id>`; the whole span is busy and every one of them stays on the Calendar
```

It is not a risk finding. Groups containing routine occurrences retain their
existing routine diagnostics.

Containment nests transitively: a block inside a block inside a block forms one
group. Pairwise conflict checks still run before clustering. Two children that
partly overlap each other inside a shared container are still a collision
between themselves; equal-span children also remain a collision.

## A collision does not cancel the day

Until P1B-54 one `static_task_collision` meant the kernel was never called.
There was no Plan at all: one double-booking on a real calendar, and nothing
else in the week was planned either. The kernel does refuse two overlapping
capacity-occupying anchors, with no candidates, so the request could not
simply be handed over.

The collision is now a **warning**, and it is planned around the way a routine
occurrence over a commitment always was:

- **every overlapping pair of capacity-occupying Statics joins one group** in
  `committed_clusters`, whether one contains the other or not. The kernel is
  handed one carrier for the group's union window, which it accepts. It is
  never handed two anchors that overlap;
- **both Tasks keep their own windows** and both are on the Calendar;
- **the whole span is busy.** No Dynamic work is placed inside it;
- the pair is still reported, once, as `static_task_collision`.

The message names both Tasks by title and by id:

```text
Static Tasks “<title>” (`<id>`) and “<title>” (`<id>`) overlap; both keep their fixed windows and stay on the Calendar, and the whole span is busy
```

A Task with no title is named by its id alone.

**A Static dependency that cannot hold is dropped, with the same code.** A
Static Task whose Static prerequisite ends after it starts could never be
satisfied: both windows are fixed. That was the second way to get
`static_task_collision` and no Plan. The edge is now dropped, both Tasks keep
their windows, and the message says so:

```text
Static Task “<title>” (`<id>`) depends on “<title>” (`<id>`), which ends after it starts; both keep their fixed windows and stay on the Calendar, and the dependency is not enforced
```

A pair that both overlaps and has such an edge is one diagnostic, the second
form, ending `; the whole span is busy`. A prerequisite that contains its
dependent, or is contained by it, is containment and not a conflict, as
before.

What is unchanged: an empty store has nothing to plan and produces no
candidates; a routine occurrence over a commitment reads
`routine_occurrence_overlaps_commitment`; containment reads
`static_tasks_share_committed_time`; and a store with no collision is handed
to the kernel exactly as it was. `POST /planning/recalculate` follows the same
rule.

`static_task_collision` is not a risk finding, and nothing blocks on it. The
double-booking is still the operator's to resolve: the warning is how they
learn of it.
