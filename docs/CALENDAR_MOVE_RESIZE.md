# Calendar move and resize

Calendar capture now imports the operator's Static window and the duration of
Dynamic work. It uses the same single bounded RecordingCalendarApi/Google list
as completion and reopen; nothing polls.

## The undo this closes

Before P1B-34, reconcile repair updated UbU's record of the Calendar event while
leaving the Task's window unchanged. Preview therefore proposed undoing a real
reschedule. The measured probe at `88b70d4` was:

```text
PROBE[drag] reconcile: drifted   `Dentist`
PROBE[drag] after repair, UbU believes it sits at 2026-09-26T16:00:00Z
PROBE[drag] next preview, with the Task's static_window unchanged:
PROBE[drag] UPDATE Dentist @ 2026-09-26T14:00:00Z
PROBE[drag] => UbU proposes moving the operator's meeting back to 2026-09-26T14:00:00Z
```

Capture now edits the Task's `static_window` through the ordinary versioned Task
edit path, records a source-marked decision Log, and accepts the observed event.
Preview uses the current Static Task window even if its stored plan predates the
edit. Regeneration is not needed to prevent the next preview undoing the move:

```text
PROBE[honour] next preview, with the Task's static_window following the drag:
PROBE[honour] operations: none
```

## Placement decides the gesture

| Placement | Colour means | Window change means |
| --- | --- | --- |
| Dynamic | Done; removing Calendar completion colour can reopen recent work | Resize: change the declared duration; placement remains the planner's |
| Static | Category | Move: make `static_window` follow the event |

Dragging Dynamic one-off work without changing its duration emits no move or resize
signal. It does not pin the Task or create a Static commitment. The next plan
still chooses its position. After reconcile repair accepts the observed event
position into the projection record, preview proposes the planner's slot again.
Capture itself continues to report this unresolved position drift.

A Dynamic resize writes a fixed `duration_estimate` at the current Task version.
It does not rewrite the stored plan or any placement field; the next generate
uses the new duration. If the work no longer fits, the existing partial placement
machinery reports the outcome.

A one-off resize changes what the operator **declared**. Routine occurrences
now have a separate P1B-35 path: any changed Calendar window writes a per-date
override and pins that span, preserving the routine's duration declaration.
P1B-23 observations still size unpinned Planned occurrences; a pinned override
uses its explicit span instead.

## Occurrences and combined gestures

Routine occurrence payloads are rebuilt on materialize. P1B-35 stores their
window override on the Objective schedule, keyed by local date, so it survives
that rebuild without changing occurrence keys. This replaces the former
occurrence-drag rejection. Direct occurrence field edits remain rejected.

Window edits run before completion. Under the placement partition a Static
move never also completes the Task: its colour means category. A Dynamic drag
that changes duration and colour applies the resize before completion, and the
completion Log records the observed dragged window. A Dynamic position-only
drag plus colour completes the work without a window edit. This is the literal
partition used to resolve judgment call 9's otherwise incompatible wording.

A changed window on any non-active Task produces
`calendar_gesture_on_inactive_task`; neither move nor resize revives work.
The separate, explicitly bounded P1B-33 colour-removal undo remains available.

## Sanity guards and reporting

A Static move is rejected without a Task edit or move Log when:

- The span is non-positive or its timestamps are invalid:
  `calendar_move_invalid_window`.
- It starts before the Task's `provenance.created_at`:
  `calendar_move_before_creation`.
- Its end is later than the current planning horizon's end plus one configured
  `planning_horizon_seconds` length: `calendar_move_beyond_horizon`.

A missing creation timestamp also prevents the move. Bounds are inclusive at
creation and at the maximum end. They guard against corrupt data and do not
resolve scheduling conflicts. The capture list remains bounded: an event moved
entirely outside that observation window is not seen in this pass.

`moved` and `resized` count successful edits. They are detail counts within
`updated`, which counts each changed Task once even when resized and completed
together. Rejected gestures have zero edit counts and visible diagnostics;
existing capture, unchanged and skipped accounting remains in place.

## Historical P1B-34 limits

These limits record P1B-34. P1B-35 supersedes the occurrence and observed-resize
restrictions above; Dynamic one-off placement remains owned by the planner.


1. **Routine occurrences cannot be moved from the calendar.** The occurrence override does not exist; P1B-35 builds it. Until then the gesture is rejected visibly.
2. **A resize may be overridden.** P1B-23's observed model wins for a well-observed routine. The diagnostic says so; the behaviour does not change.
3. **Dynamic position is not honoured, by design.** Dragging Dynamic work moves it back on the next apply.
4. **On demand.** Nothing polls, so a drag lands at the next capture.
5. **No conflict resolution between gestures and the app.** If a Task is edited in the app and its event dragged before the next capture, last write wins, and the calendar is usually last.
6. **A move does not adjust neighbours.** Dragging a meeting onto other work creates an ordinary planning conflict, reported by the existing machinery, not resolved here.
