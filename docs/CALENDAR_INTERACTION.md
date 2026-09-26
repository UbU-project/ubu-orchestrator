# Calendar interaction: colour it done

P1B-33 corrects P1B-28's unconditional projection of category colours. The reference implementation, `quick-ubu/gcal` at `9ccc8b8`, exports colour only for pinned work:

```rust
color_id: if visible && task.pinned.is_some() {
    task.category.as_ref().and_then(|category| color_map.get(category)).cloned()
} else {
    None
},
```

| | exported with | a colour on the phone means |
|---|---|---|
| unpinned / Dynamic | **no colour** | **done** |
| pinned / Static | its category colour | its category |

Mainline uses `ScheduledTaskBody.static_anchor` at step construction. `TaskDisplay` keeps the palette's resolved colour and every step retains its `category_tag`; only Static steps receive `gcal_color_id`. Direct Static steps retain their existing colours. Dynamic work loses its category colour on the phone, buying the familiar one-tap completion gesture.

## Complete from the phone

After generating and applying an uncoloured Dynamic event, give it **any colour** on the phone. Run the existing capture pass:

```http
POST /projection/calendar/capture
Content-Type: application/json

{"schema_version":"ubu.orchestrator.calendar_capture.v1","export_mode":"live"}
```

Use the existing credential-path configuration and explicit process enablement for live mode. Mock tests use only `RecordingCalendarApi`. There is no new endpoint, background poll, or second Calendar read.

Only applied ownership permits interaction. A foreign event still follows P1B-32 capture; a known-but-unrecorded event is not a completion signal. An owned Dynamic event coloured by the operator completes its Task through the ordinary recorded action machinery, including lifecycle validation, `apply_completed_effects`, cold-store UniverseState seeding, and a canonical Log entry. Its payload adds:

```json
{"source":{"source_kind":"google_calendar","source_id":"aaaaa"},"observed_window":{"start":"2026-09-25T09:05:00Z","end":"2026-09-25T09:50:00Z"}}
```

The example ID is synthetic. The window is what capture **observed**, not the original plan. Capture uses its existing canonical UTC whole-second representation. It does not change a Task's duration declaration or make a Dynamic Task Static.

The ticket refers to `/task/:id/done`, but this repository's legacy endpoint only appends a `task_done` intent fact. Canonical completion and effects live in `/task/:id/action` with `action: "complete"`. Calendar completion reuses that path's implementation; it does not create a second completion engine or change the legacy endpoint.

A successful gesture updates the applied snapshot to the accepted observation, without a Calendar write. Repeated capture of the same completed coloured event does not complete it again, apply effects again, or append another Log entry. Projection retains calendar-completed events, including after regeneration removes the Task from the plan, so it does not erase the gesture or delete the event needed for reopening. This retention has no automatic history cleanup.

## Reopen only a Calendar completion

Quick UbU's comment explains the guard:

> Only undo a Calendar completion (which records an actual window), not a CLI `done` fact with no window.

Removing a colour reopens only when all of these hold:

- The event is owned and the Task is Dynamic and not captured.
- The Task is currently completed and the observed event has no colour.
- The **most recent completion** Log entry names this Calendar event through the source marker and contains a valid observed window.
- The observed event end is strictly later than `now - 24h`. Exactly 24 hours old does not qualify.

The strict comparison reproduces Quick UbU. It imposes no upper bound of `end <= now`; future-ended events also satisfy the reference rule. The lookback keeps old uncoloured history from reopening finished work. Completion entries are ordered by append order, including legacy app `task_done` facts; equal timestamps cannot let an older Calendar completion overrule a newer app completion.

Reopen restores active status through shared canonical admission and appends a `decision_recorded` Log with `action: "reopen"`, `decision: "task_reopened"`, and `completion_log_id`. Its object references include both Task and completion. No suitable existing reopen action existed, so this is a narrow internal action, with no new public action enum or endpoint. It rechecks status and the most recent completion under the same action lock used by app actions. A newer app completion is never undone.

The reopen requested here restores lifecycle state and records the undo link. It does not invert UniverseState effects: arbitrary effect mutations have no general inverse in this action path. Completing the Task again runs the ordinary effects again. Existing completion/admission/effect/Log writes remain separate store operations; this ticket does not add a new transaction model.

## One bounded observation

Capture makes one paginated list request spanning the planning horizon and its recent-owned-event lookback: start is the earlier of the planner's start and `now - 24h`, end remains the planning horizon end. This makes recent past events reachable even when the default planning horizon starts now. The ordinary planner horizon remains the bound for **foreign** capture; unowned history in the lookback is ignored. No unbounded historical read is made.

Static and captured events produce neither completion nor reopen signals. Their colours remain category metadata. An unresolved owned edit is reported as drift; moving or resizing Static/captured work is P1B-34. Reconciliation/repair can record the observed drift so ordinary preview/apply restores the canonical plan; capture itself sends no Calendar writes.

## Corrected capture counts

The response retains schema `ubu.orchestrator.calendar_capture.v1` and adds the fourth integer bucket:

```json
{"captured":0,"updated":0,"unchanged":1,"skipped":0,"diagnostics":[]}
```

| Bucket | Meaning |
|---|---|
| `captured` | Newly admitted foreign-source Tasks |
| `updated` | Existing Tasks whose canonical state actually changed, including completion/reopen |
| `unchanged` | Owned observations matching the applied snapshot; also source recovery with an unchanged Task payload |
| `skipped` | Entries that could not be captured, such as all-day or known-but-unrecorded events |

An unresolved owned difference produces `capture_owned_drift` naming the Task and event, with neither an update nor a skipped count. A successfully consumed completion/reopen is an update, not unresolved drift. Pure detection stays silent for Static/captured Tasks; the enclosing capture pass can still report their unrelated drift.

P1B-32 used `updated` for unchanged captured-source reuse and `skipped` for owned drift (and untouched ordinary projected events). Thus an older `updated: 40, skipped: 3` report can mean forty untouched commitments and three owned differences. P1B-33 instead reports `unchanged: 40`, with named drift diagnostics. No second Task admission occurs for unchanged reuse. Older verification reports preserve their historical results; these are the current meanings.

## Existing coloured Dynamic events

Old P1B-28–32 applied snapshots can already contain Dynamic category colours. They are not operator completion gestures: completion requires an applied baseline that UbU left uncoloured. Regenerate, preview, and apply the new partition to clear those legacy Dynamic colours before using the gesture. Capture phone gestures before accepting them as generic drift through reconciliation repair; repair replaces the applied baseline with its observation.

## Known limits (verbatim)

1. **Dynamic work has no category colour on the phone.** This is the price of the completion gesture, and it is the trade Quick UbU already makes.
2. **Move and resize are not handled.** Dragging or stretching an event changes nothing in UbU. That is P1B-34, and until then a moved Static event shows as `drifted` in reconcile and is repaired back.
3. **Any colour completes.** Which colour is not interpreted, so a stray colour on a Dynamic event completes its Task.
4. **On demand.** Nothing polls, so a completion made on the phone lands only at the next capture.
5. **Reopen is bounded and one-deep.** Only the most recent completion can be undone, and only within 24 hours.
6. **A resize would fight P1B-23.** When P1B-34 adds it, note that an edited `duration_estimate` is a declaration, and P1B-23 overrides the declaration once five observations exist — so a manual resize may have no visible effect on a well-observed routine.

