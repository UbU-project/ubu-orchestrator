# Static containment

For two capacity-occupying Static Tasks without routine-occurrence exceptions:

| Window relationship | Result |
| --- | --- |
| Proper containment: one window encloses the other, with at least one different edge | Share committed time; keep both Tasks on the Calendar |
| Equal start and end | Blocking `static_task_collision` |
| Partial overlap: neither window contains the other | Blocking `static_task_collision` |

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
`static_tasks_share_committed_time`, counting members other than the carrier:

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
