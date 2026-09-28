# Task decomposition

DESIGN §9.4 and UBU-D0278 require a structural replacement, ordered children,
contiguous segments, lineage, derived completion, and an admitted undo that
preserves history. This implementation delivers those behaviors except for
the explicit known limits below. UBU-D0279 treats each compiled segment as
one atomic placement unit.

The original probe demonstrated correct ordering without a useful checklist:

```text
PROBE[after] origin present = false
PROBE[after] order = ["Buy hinges", "Remove old hinges", "Hang the gate"]
```

```text
PROBE[after] 2026-09-27T09:00:00Z Buy hinges
PROBE[after] 2026-09-27T15:00:00Z Remove old hinges
PROBE[after] 2026-09-27T15:30:00Z Hang the gate
```

## Admission and handles

`POST /task/:task_id/decompose` accepts:

```json
{
  "schema_version": "ubu.orchestrator.container.v1",
  "expected_version": 1,
  "children": [
    {"title":"Buy hinges","duration_estimate":{"type":"fixed","seconds":1800}},
    {"title":"Remove old hinges","duration_estimate":{"type":"fixed","seconds":1800}},
    {"title":"Hang the gate","duration_estimate":{"type":"fixed","seconds":1800}}
  ],
  "segment_split_points": []
}
```

The operation mints new handles for every child and for the Container. The
original Task becomes moot with `replaced_by_new_plan_structure`; its handle
never changes type and is never reused. Children inherit only its compartment.
The original Task and the snapshot in the mutation Log retain the intent,
Objective reference, prior schedulable fields and dependency context for audit
and restoration.

One batch admits the Log, children in order, Container, and moot origin.
The first Log and final origin mutation observe the requested origin version.
A stale or concurrent write yields HTTP 409 with `version_conflict`, with no
partial writes. Each child is chained to its predecessor in addition to its
explicit dependencies.

Proposed siblings have no canonical IDs yet. With the operator's approval,
`blocked_by: ["child:2"]` refers to the second child in the proposal (one-based);
the service resolves it before admission. Ordinary Task IDs are still accepted.
Forward and self references are rejected. Proposed child names in admission
messages use one-based positions and titles. A decomposition child cannot itself
be decomposed.

## Boundaries

Split points are exclusive indices. Four children `[A,B,C,D]` with `[2]` form
`[A,B]` and `[C,D]`. Both pairs stay contiguous; unrelated work can occur
between the pairs. No split points means one segment. Response positions and
diagnostic segment indices are zero-based.

| Admission diagnostic | Required correction |
| --- | --- |
| `decompose_routine_occurrence_unsupported` | Edit the routine template; a boundary cannot make an occurrence decomposable. |
| `decompose_inactive_task` | Choose active work; a boundary cannot change lifecycle state. |
| `decompose_needs_children` | Supply at least two children. |
| `decompose_invalid_split_points` | Use distinct increasing integer indices inside the list. |
| `decompose_child_order_conflict` | Reorder children or revise the dependency; a split alone cannot reverse checklist order. |
| `decompose_interior_precondition_needs_boundary` | Put boundaries around the gated child so it is a singleton. |
| `decompose_static_child_needs_boundary` | Put boundaries around the Static child so it is a singleton. |
| `decompose_segment_bounds_disjoint` | Separate incompatible time ranges or deadlines with boundaries, or revise the bounds. |

Allowed ranges intersect and the earliest deadline bounds the whole multi-child
unit. The intersection must fit the placement duration. This is deliberately
conservative: individual offsets could fit some schedules that this whole-unit
intersection rejects. Effects remain ordinary child completion effects.

Routine occurrences cannot be decomposed because materialization rebuilds them
from their templates. Otherwise the original could return while its children
were orphaned.

## Compilation and expansion

The kernel contract does not change. Before dispatch the orchestrator replaces
each eligible multi-child segment with its first child's Task ID as carrier.
It retains every child spec beside the request for expansion.

Fixed durations sum exactly. Mixed durations use a component-wise sum of minimum,
mode and p95; a Fixed child contributes its seconds to every component. At least
one stochastic child preserves strict `min < mode < p95`. Checked arithmetic and
contract validation report `container_segment_uncompilable` if a valid model
cannot be formed. Incompatible later edits also leave a segment uncompiled with
that diagnostic.

The unit intersects windows, takes the earliest member deadline, unions external
dependencies and mandatory flags, takes the maximum child value, and unions
correlation groups with the greatest strength for each group. Internal edges are
redundant and removed. Outside edges to removed members point to the carrier.
Child preference values are retained through compilation so a less valuable
carrier cannot lower the whole checklist's omission value.

After placement, both candidate and canonical Plan paths expand the carrier into
real child Task IDs with their own titles and categories. The identity
`placement_seconds(unit) == sum(placement_seconds(child))` holds for Fixed
seconds and stochastic modes. Code asserts the last child's end equals the
carrier's end exactly. Calendar projection and Plan storage receive ordinary
Task entries.

Missing prefix children are historical: compile the remaining unstarted suffix.
Fewer than two remaining children need no compilation. A hole inside the
eligible membership emits `container_segment_partial` with Container, segment
and missing IDs and leaves that segment uncompiled. The scatter guard checks
placed members after Plan assembly and emits `container_segment_scattered`
with the largest gap in whole minutes if adjacency ever fails.

A segment is harder to place than its children were separately. Ninety free
minutes spread over three thirty-minute gaps cannot hold a ninety-minute unit.
That cost is the point of requiring contiguous work. When a carrier is unplaced,
every member appears with its own title and ID, the kernel reason, the Container
and segment, the required contiguous seconds, and the remedy: add a split point.
The same expansion applies to pre-kernel exclusions.

## Undo and listing

`POST /container/:container_id/undo` requires the same schema version. One batch
records `container_decomposition_undone`, admits a new restored Task, moots every
non-completed child, and supersedes the Container with
`superseded_by_task_ref`. The restored Task carries the original intent and
valid schedulable fields. Expired timing constraints are omitted. The original
Task stays moot and unchanged. Completed children remain byte-for-byte intact
and are reported separately. Version preconditions cover all observed children,
including the completed ones, so a concurrent completion cannot be erased.

UBU-D0259's decomposition sentence describes the older un-tombstoning model.
UBU-D0278 governs this implementation; the design-document reconciliation is
outside this ticket.

`GET /containers` lists all Containers without pagination, including superseded
ones: IDs, titles, status, origin, ordered children, segment ranges and derived
completion. Completion is `complete` exactly when every child is completed or
moot; otherwise it is `in_progress`. It is computed when reading rather than
stored, avoiding stale summary state after child transitions.

## Known limits


1. **No nesting.** A child cannot itself be decomposed into a further Container. §9.4 does not forbid it; nothing here supports it.
2. **One rollout sample per segment.** `UBU-D0278` specifies the conservative component-wise sum, and accepts that it is conservative. Native no-gap kernel edges stay deferred until measurement shows the approximation distorts rollouts.
3. **Repair does not re-derive the suffix mid-run.** §C's suffix rule applies at plan time. `UBU-D0278`'s repair behaviour — replanning from the next segment boundary when a suffix can no longer satisfy its constraints — is not implemented; the ordinary repair path runs instead.
4. **Children inherit only the compartment.** Deadlines, categories and estimates are per child, stated by the operator.
5. **The Container is invisible to planning except as segments.** Nothing can prefer or defer a checklist as a unit; Preferences and values apply to children individually, and a segment takes the maximum.
6. **No suggestion route.** The operator writes the children. Quick UbU's LLM-assisted decomposition has no mainline equivalent yet; `UBU-D0278`'s `Decomposition` advisory candidate shape is not implemented, and the advisory queue is where one would land.
7. **Undo is one level and one step.** There is no history of decompositions, and undoing after a child has itself been restructured is not modelled.
8. **External References are not rewritten.** §9.4's rule for linking an external object to the Container or to a specific child is not implemented; existing references stay on the origin.
9. **A segment is harder to place than its children were.** Compiling trades placeability for contiguity, on purpose. The operator's remedy is a split point, and §D's unplaced expansion is what tells them so.
10. **Split policy does not exist yet.** `UBU-D0284` requires that a child needing independent splitting inside a multi-child segment force a boundary or be rejected. There is no split policy on a Task today, so there is nothing to reject; when `UBU-D0284` lands, §B gains that rejection.
11. **`UBU-D0259` and `UBU-D0278` disagree about undo in `ubu-design`.** The operator has decided `UBU-D0278` governs and that `UBU-D0259`'s decomposition-undo rule is deprecated. This ticket follows `UBU-D0278`; the document edit making the deprecation explicit is still pending.

## Verification

Write `docs/P1B-37_VERIFICATION.md` recording:

- the test count, the clippy delta with the counting method stated, that `Cargo.lock` is byte-identical and that no pin moved;
- **the verbatim stored Container record** from test 1, so every §9.4 field is on the record, and the verbatim store deltas proving the batch was one transaction;
- **the verbatim plan from test 2**, with the start and end of each step, beside the six-hour probe output it replaces;
- the verbatim compiled `TaskSpecBody` that reached the kernel in test 5, and the expanded steps beside it;
- confirmation from test 6 that nothing landed, and what you asserted to establish it;
- the verbatim rejection from each of the eight cases in test 7;
- the verbatim `container_segment_partial` from test 9;
- **the verbatim before and after of test 11** — the origin id, the restored Task id, the Container's status and `superseded_by_task_ref`, and each child's status;
- confirmation that `container_segment_scattered` fired in no test, and where you asserted it;
- **the verbatim `unplaced_tasks` entries from test 15**, all three of them, beside the placements the same three children get when they are not one segment;
- which of the sixteen judgment calls you disagree with, if any, and why;
- the eleven known limits, and anything ambiguous with the literal reading you took.
