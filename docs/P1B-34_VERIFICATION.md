# P1B-34 verification

Historical checkpoint. P1B-35 supersedes the occurrence rejection and updates
two tests. The original evidence remains in this file at commit `95dec6e`.

Base: `88b70d46bcb502a46d94af9536e718038d0994d7` (clean).
Branch: `p1b-34-calendar-move-resize`. One signed commit per section A–F.
Only `ubu-orchestrator` changes. No dependency added and no pin moved.

## Results and method

- **283 passed, 0 failed, 0 ignored**, including exactly eleven new integration
  tests in `tests/calendar_move_resize.rs`. This is 272 + 11. The full suite
  also passed with 272 tests at each section A–E checkpoint.
- Clippy: **10 before, 10 after, delta 0**. The warning sets are identical.
  Both measurements used `cargo clippy --locked --offline --all-targets
  --message-format=json` with the same sanitized environment. Count distinct
  `compiler-message` warnings by `(code, message, primary spans' file, line,
  column)`, deduplicating repeated lib/lib-test messages. There are no added
  or removed warnings, even by exact source location.
- `Cargo.lock` is byte-identical to the clean baseline, not merely equivalent.
  SHA-256: `4a73890bbac0991eaad68d129e5131e945336c28cf806b859369dbbf963879f9`.
- Every sibling repository's HEAD and status was compared to its baseline;
  all eleven remain unchanged and clean. In particular: core `72b3ad8`, schemas
  `d12835f`, store `eeacb6e`, planning-kernel `cf8b2ca`, GitHub adapter `963d2c8`,
  Quick `9ccc8b8`, design `f7c4a1d`.
- OpenAPI was regenerated with `cargo run --locked --offline --example
  generate_openapi`; its only changes are the two new response count fields.
- Only the new `tests/calendar_move_resize.rs` was passed to `rustfmt
  --edition 2021`. No repository-wide formatter was run.
- No existing diagnostic code string was removed or renamed.

The existing P1B-33 test
`phone_colour_completes_with_observed_window_and_source_log` was updated in
section C: its observed 45-minute event now expects a fixed **2700-second**
estimate instead of the old 1800-second declaration. This is the one existing
test expectation changed by this ticket; its completion/source/effects checks
remain intact. The new model test initially assumed a 09:00 placement; that
fixture assumption was corrected to assert the model's 1200-second duration,
leaving placement to the planner. All final runs pass.

The new regression assertions also verify that equivalent UTC-offset windows
do not create spurious move admissions, that repeated capture converges, that
neither guard rejection nor occurrence rejection edits a Task, that an app edit
creates no fake Calendar Log, and that a mixed capture lists events only once.

## Offline evidence

Every test invocation used:

```sh
env -i PATH="$PATH" HOME="$HOME" CARGO_NET_OFFLINE=true \
  python3 /tmp/p1b-34-results/offline-test.py cargo test --locked --offline
```

The wrapper installs an inherited libseccomp filter before executing Cargo:
`SCMP_ACT_ALLOW` by default, with `SCMP_ACT_ERRNO(EPERM)` on `socket` when its
first argument is `AF_INET` (2) or `AF_INET6` (10). All child processes inherit
it. Normal Cargo and compiler concurrency is unchanged. The final full run
also traces every descendant's network syscalls:

```sh
env -i PATH="$PATH" HOME="$HOME" CARGO_NET_OFFLINE=true \
  python3 /tmp/p1b-34-results/offline-test.py \
  strace -f -qq -e trace=network -o /tmp/p1b-34-results/F-network.log \
  cargo test --locked --offline -- --nocapture
```

The final trace contains **488 lines, 0 AF_INET/AF_INET6 occurrences,
0 `connect(` calls**. Local AF_UNIX compiler IPC is not an external network
call. No test attempted an Internet socket or connection. The new tests inject
`RecordingCalendarApi` and use in-process HTTP requests; no Google transport or
real credentials, Calendar IDs, event IDs or Quick data are used. Evidence below
is copied verbatim from that successful full run, without rewriting IDs.

## Test mapping

| Ticket | New integration test | Verified outcome |
| --- | --- | --- |
| 1 | `captured_meeting_follows_drag_and_next_preview_has_no_operation` | Captured Dentist window follows drag; immediate preview and regenerated preview have no operations; repeat capture converges |
| 2 | `reconcile_still_classifies_unimported_drag_as_drifted` | Classification remains drifted before capture and matched afterwards |
| 3 | `dynamic_resize_changes_declaration_without_changing_planned_position` | Fixed 2700-second declaration; current planned position untouched; next plan uses new duration |
| 4 | `dynamic_position_alone_is_not_a_signal_and_repair_projects_planned_slot` | No signals; after repair, preview restores planner position |
| 5 | `occurrence_drag_requires_override_and_does_not_edit_task` | Named Task/routine diagnostic, unchanged Task |
| 6 | `observed_routine_resize_reports_model_priority_without_bypassing_override` | Five observations, both occurrence/model diagnostics, unchanged declaration and 1200-second model duration |
| 7 | `all_three_move_sanity_guards_reject_without_writes` | Zero span, pre-creation start and excessive end rejected without Task, Log or admission writes |
| 8 | `completed_task_gesture_is_ignored_visibly` | Inactive diagnostic; unchanged completed Task and Logs |
| 9 | `dynamic_drag_resize_and_colour_apply_before_completion_with_observed_window` | Resize then completion (version 1 → 3), observed window on completion, unique updated count; Static colour remains category |
| 10 | `calendar_move_log_has_source_marker_but_app_edit_does_not` | Move Log carries source; app edit adds no fake Calendar Log |
| 11 | `mixed_capture_reports_moves_resizes_and_unique_updated_tasks` | moved=1, resized=1, updated=2, unchanged=1, one list |

## Verbatim evidence

Test 1 first reproduces the pre-capture undo: reconcile reports drift, repair
accepts the observed 16:00 event, and the still-14:00 Task produces an UPDATE.
It then runs capture, checks the canonical 16:00 window, and previews immediately
without regenerate. Test 2 separately covers direct drag → capture without
repair. These cover both entrances to the fix.

### Test 1 — before capture: canonical window and preview operations

```json
{"operations":[{"event":{"color_id":null,"end_at":"2026-09-26T14:30:00Z","external_id":"5n0q8c9h7g4k2m1p3r6t8v0a2c","reminders_minutes":[],"start_at":"2026-09-26T14:00:00Z","summary":"Dentist","task_id":"task_01a0dda9e4d871d1973945a8580d9b30","transparent":false},"kind":"update"}],"static_window":{"end":"2026-09-26T14:30:00Z","start":"2026-09-26T14:00:00Z"}}
```

### Test 1 — after capture: canonical window and immediate preview operations

```json
{"operations":[],"static_window":{"end":"2026-09-26T16:30:00Z","start":"2026-09-26T16:00:00Z"}}
```

### Test 4 — no gesture signals, capture drift, preview after repair

```json
{"diagnostics":[{"code":"capture_owned_drift","message":"Owned Task `task_01a0dda9e4db76c0b5c1ad1deca5f2e6` event `01a0dda9e4db76c0b5c1ad1deca5f2e6` differs from the applied record; no Task update was made"}],"operations":[{"event":{"color_id":null,"end_at":"2026-09-26T08:30:00Z","external_id":"01a0dda9e4db76c0b5c1ad1deca5f2e6","reminders_minutes":[],"start_at":"2026-09-26T08:00:00Z","summary":"Synthetic Dynamic work","task_id":"task_01a0dda9e4db76c0b5c1ad1deca5f2e6","transparent":false},"kind":"update"}],"signals":{"diagnostics":[],"moves":[],"resizes":[]}}
```

### Tests 5 and 6 — historical occurrence rejection

The retired rejection evidence is preserved in the
[original P1B-34 verification](https://github.com/UbU-project/ubu-orchestrator/blob/95dec6ec1d60ce20375e878a641d79a1cef6c4bc/docs/P1B-34_VERIFICATION.md).
P1B-35 replaces those two tests with accepted date-override behavior; the old
verbatim output has not been rewritten to pretend it describes current behavior.

### Test 7 — zero-length window

```json
[{"code":"calendar_move_invalid_window","message":"Task `task_01a0dda9e4e274828041f6c6d4eef097` event `01a0dda9e4e274828041f6c6d4eef097`: window must have a strictly positive span; move rejected"},{"code":"capture_owned_drift","message":"Owned Task `task_01a0dda9e4e274828041f6c6d4eef097` event `01a0dda9e4e274828041f6c6d4eef097` differs from the applied record; no Task update was made"}]
```

### Test 7 — start before creation

```json
[{"code":"calendar_move_before_creation","message":"Task `task_01a0dda9e4e274828041f6c6d4eef097` event `01a0dda9e4e274828041f6c6d4eef097`: window starts before Task creation; move rejected"},{"code":"capture_owned_drift","message":"Owned Task `task_01a0dda9e4e274828041f6c6d4eef097` event `01a0dda9e4e274828041f6c6d4eef097` differs from the applied record; no Task update was made"}]
```

### Test 7 — excessive end

```json
[{"code":"calendar_move_beyond_horizon","message":"Task `task_01a0dda9e4e274828041f6c6d4eef097` event `01a0dda9e4e274828041f6c6d4eef097`: window ends beyond the planning horizon plus one configured horizon length; move rejected"},{"code":"capture_owned_drift","message":"Owned Task `task_01a0dda9e4e274828041f6c6d4eef097` event `01a0dda9e4e274828041f6c6d4eef097` differs from the applied record; no Task update was made"}]
```

### Test 10 — complete source-marked move Log

```json
{"created_at":"2026-09-26T08:00:00Z","event_type":"decision_recorded","id":"log_01a0dda9e5157eb288a70e846bc80cd8","object_refs":["task_01a0dda9e4e175a396d01336093a8368"],"payload":{"action":"move","decision":"task_window_changed","schema_version":"ubu.orchestrator.calendar_interaction.v1","source":{"source_id":"01a0dda9e4e175a396d01336093a8368","source_kind":"google_calendar"},"static_window":{"end":"2026-09-26T16:30:00Z","start":"2026-09-26T16:00:00Z"}},"provenance":{"authority_source":"user","created_at":"2026-09-26T08:00:00Z"}}
```

## Judgment calls and literal interpretations

Agree with judgments **1–8 and 10**, and with judgment 9's intent that simultaneous
window and completion gestures both apply with the observed completion window.
Judgment **9 cannot literally make a Static move also complete a Task** while
preserving the mandated placement partition. “Move first” is interpreted as
window edit first: test 9 applies a Dynamic resize before completion. A Static
move with colour stays active; a Dynamic position-only drag with colour has no
window edit. There is no change to P1B-33's colour partition.

At this historical checkpoint, judgments 3 and 4 intersected: routine
occurrences rejected both window gestures even when an observed duration model
existed. P1B-35 replaces that behavior with canonical per-date window overrides.
The original judgment discussion is preserved at the revision linked above.

Other bounded interpretations:

- “Next preview” for Static is immediate, even with an older admitted plan.
  Preview reads current non-occurrence Static Task windows. It does not generate
  a new plan or adjust neighbours. The next generate uses the edited Task too.
- Dynamic position-only capture preserves the existing unresolved-drift report.
  As in the ticket's measured probe, test 4 repairs projection bookkeeping before
  checking the UPDATE to the planner's slot. Capture does not silently repair
  an ignored position gesture.
- Window comparisons use parsed instants, so UTC offsets do not create edits.
  Canonical projection coordinates remain whole UTC seconds.
- For a non-fixed declared estimate, the declared nominal duration is its
  `mode_seconds`; legacy/missing estimates use the planner's existing nominal
  duration fallback. A successful resize replaces it with a fixed estimate.
- Move limits use the current resolved planning horizon's end plus one configured
  horizon length; they do not add another length of an arbitrary request window.
  Capture still only sees events overlapping its bounded observation range.
- Inactive diagnostics require a changed window; merely observing an unchanged
  completed event is not another gesture. The independent Calendar colour-removal
  reopen rule remains unchanged.
- `moved` and `resized` are successful-edit detail counts within unique `updated`
  Tasks. Rejected gestures are diagnosed, not counted as successful edits.
- Move admission and its decision Log use the existing separate admission/Log
  boundaries; this ticket does not introduce a cross-operation transaction.

## Known limits

1. **Routine occurrences cannot be moved from the calendar.** The occurrence override does not exist; P1B-35 builds it. Until then the gesture is rejected visibly.
2. **A resize may be overridden.** P1B-23's observed model wins for a well-observed routine. The diagnostic says so; the behaviour does not change.
3. **Dynamic position is not honoured, by design.** Dragging Dynamic work moves it back on the next apply.
4. **On demand.** Nothing polls, so a drag lands at the next capture.
5. **No conflict resolution between gestures and the app.** If a Task is edited in the app and its event dragged before the next capture, last write wins, and the calendar is usually last.
6. **A move does not adjust neighbours.** Dragging a meeting onto other work creates an ordinary planning conflict, reported by the existing machinery, not resolved here.
