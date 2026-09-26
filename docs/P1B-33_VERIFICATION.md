# P1B-33 verification

Base `0b55fb78753fc2989e962bb5addc7593dde7bfe9` was clean before editing. Branch: `p1b-33-calendar-completion`. Only `ubu-orchestrator` changes. Sections A–F each have their own commit; no force-push or pin chain.

## Results and method

**272 tests pass; zero failures, zero ignored.** Baseline: 262. Exactly ten new tests are in `tests/calendar_interaction.rs`. Full offline suites passed before each section commit: A–E each 262, F 272. The interrupted turn was recovered by checking the branch and completed section C test log before committing; completed actions were not repeated.

All test invocations used a sanitized environment, Cargo offline/locked mode, and an inherited libseccomp filter denying `socket(AF_INET, ...)` and `socket(AF_INET6, ...)` with EPERM. Default action is ALLOW, so local IPC continues to work. Final full-suite command:

```sh
env -i PATH="$PATH" HOME="$HOME" CARGO_NET_OFFLINE=true \
  python3 /tmp/p1b-33-results/offline-test.py \
  strace -f -qq -e trace=network -o /tmp/p1b-33-results/F-network.log \
  cargo test --locked --offline -- --nocapture
```

The final trace contains **489 lines, zero AF_INET/AF_INET6 occurrences, and zero `connect` calls**. Recorded socket activity is local compiler IPC. No test performed an Internet/network connection. Calendar operations use `RecordingCalendarApi`; HTTP tests use Axum `oneshot` in process. Fixtures, event IDs, Task IDs and UniverseState data are synthetic. No live Calendar, OAuth credentials, real event IDs or real Quick UbU data were used. No credentials were committed, logged, returned or stored. Compiler/Cargo concurrency remained at its existing settings; no execution caps were added.

Clippy used this identical command before and after in this ticket run:

```sh
env -i PATH="$PATH" HOME="$HOME" CARGO_NET_OFFLINE=true \
  cargo clippy --locked --offline --all-targets --message-format=json
```

Counting method: JSON `compiler-message` records for package `ubu_orchestrator`, level `warning`, deduplicated across targets by `(code, message, primary-span file, line, column)`. **10 before, 10 after; delta 0.** The deduplicated sets are identical. The new predicate was named `is_from_calendar_event` to satisfy Clippy's method naming convention; no warning suppression was added.

`Cargo.lock` is byte-identical to the pre-edit copy. SHA-256:

```text
4a73890bbac0991eaad68d129e5131e945336c28cf806b859369dbbf963879f9
```

`Cargo.toml` also has no diff. No dependency was added, no pin moved, and all eleven sibling repositories retain their baseline HEAD and clean tree. Required read-only pins: core `72b3ad8`, schemas `d12835f`, store `eeacb6e`, kernel `cf8b2ca`, GitHub adapter `963d2c8`, Quick UbU `9ccc8b8`, design `f7c4a1d`.

OpenAPI was regenerated under the same offline socket-denial wrapper with `cargo run --locked --offline --example generate_openapi`. `CalendarCaptureResponse` includes required integer `unchanged`. Only this ticket's new Rust files, `src/services/calendar_interaction.rs` and `tests/calendar_interaction.rs`, were passed to `rustfmt --edition 2021`; both pass its file-specific check. `git diff --check` passes.

## Every existing expectation changed by section A

Before editing, `rg -n 'gcal_color_id|color_id' tests` searched all tests. Three existing tests change expectations. Their other assertions remain, including categories, transparency and reminders.

1. `tests/calendar_projection.rs::three_routines_project_colors_transparency_reminders_and_start_order`

Before:

```rust
("Breakfast", "3", false, json!([0])),
("Work Time", "9", true, json!([])),
("Standup", "6", false, json!([10, 0])),
```

After:

```rust
("Breakfast", Value::Null, false, json!([0])),
("Work Time", Value::Null, true, json!([])),
("Standup", Value::Null, false, json!([10, 0])),
```

These fixtures are all Dynamic routine occurrences. The event's `color_id` assertion now expects null for each.

2. `tests/static_category.rs::palette_override_merges_defaults_and_matches_case_exactly`

Before:

```rust
assert_eq!(
    step["gcal_color_id"].as_str(),
    state
        .inner()
        .category_palette
        .color(step["category_tag"].as_str())
);
```

After:

```rust
assert!(step.get("gcal_color_id").is_none());
assert!(state.inner().category_palette.color(step["category_tag"].as_str()).is_some());
```

The Dynamic steps omit projected colour. The preceding exact palette assertions still verify `commute → 6`, `Commute → 8`, `custom → 6`, `work → 9`; category resolution is preserved.

3. `tests/static_category.rs::admitted_add_tag_preserves_explicit_category_and_colour`

Before:

```rust
assert_eq!(step(&cal, &task)["gcal_color_id"], "7");
```

After:

```rust
assert!(step(&cal, &task).get("gcal_color_id").is_none());
assert_eq!(state.inner().category_palette.color(Some("commute")), Some("7"));
```

The admitted tag edit still preserves `category_tag: "commute"` and the palette's resolved colour. This Dynamic Task no longer projects that colour. No existing Static colour assertion changed.

Section D separately updates two P1B-32 count tests: unchanged captured-source reuse changes from `updated: 1` to `updated: 0, unchanged: 1`; owned drift changes from `skipped: 1` to `skipped: 0, unchanged: 0` plus `capture_owned_drift`; an untouched ordinary projected event changes from `skipped: 1` to `skipped: 0, unchanged: 1`. Known-but-unrecorded events still count as skipped. No existing diagnostic code was removed or renamed. Relevant existing diagnostic codes were searched repository-wide, including tests, before refactoring the Log path.

## Verbatim evidence from the final full-suite run

The JSON below is copied unchanged from `EVIDENCE[P1B33_...]` output. IDs come from synthetic ordinary admissions and differ on rerun.

Test 1 asserts a Dynamic event has null colour and a Static event has its category colour on the actual projection preview. The Dynamic calendar step still reports `category_tag: "personal"`. The test additionally asserts explicit null colour on PATCH and omitted colour on INSERT, preventing a clearing request from being silently omitted. A pure detection check also proves an old applied Dynamic category colour does not accidentally complete the Task during migration.

Test 2, complete Log payload:

```json
{"action":"complete","decision":"task_completed","observed_window":{"end":"2026-09-25T09:50:00Z","start":"2026-09-25T09:05:00Z"},"schema_version":"ubu.orchestrator.task_action.v1","source":{"source_id":"01a0db8d589a7bc2877566652faf304d","source_kind":"google_calendar"},"task_status":"completed","transition_applied":true}
```

The test asserts completed canonical status, one completion Log, the exact Google source marker, observed 09:05–09:50 rather than the planned window, unchanged duration declaration, and no added static window. Capture makes exactly one recorder list call. Preview, regeneration and approval preserve the coloured completed event without an operation against it.

Test 3, complete stored UniverseState change:

```json
{"after":{"captured_at":"2026-09-25T08:00:00Z","facts":{"synthetic_done":true},"id":"ustate_01a0db8d58bd7891b3715428916222c8","numeric_values":{"synthetic_completions":1.0},"provenance":{"authority_source":"user","created_at":"2026-09-26T02:31:08.735530754Z"},"schema_version":"core/universe-state/0.1","source_summary":"empty UniverseState seeded on Task completion"},"before":null}
```

The cold store starts without UniverseState. Completion seeds it and applies a fact assignment plus a numeric increment. A separate synthetic Task completed through the ordinary `/task/:id/action` API produces exactly the same facts and numeric values. The Task's low success probability does not gate effects. The stored provenance timestamp above comes from the existing store persistence path and is reproduced verbatim.

Test 4, complete completion and reopen Log entries:

```json
[{"created_at":"2026-09-25T08:00:00Z","event_type":"decision_recorded","id":"log_01a0db8d58c27a909584d70d185c6178","object_refs":["task_01a0db8d588a7081a3488619c36fbe45"],"payload":{"action":"complete","decision":"task_completed","observed_window":{"end":"2026-09-25T09:50:00Z","start":"2026-09-25T09:05:00Z"},"schema_version":"ubu.orchestrator.task_action.v1","source":{"source_id":"01a0db8d588a7081a3488619c36fbe45","source_kind":"google_calendar"},"task_status":"completed","transition_applied":true},"provenance":{"authority_source":"user","created_at":"2026-09-25T08:00:00Z"}},{"created_at":"2026-09-25T10:00:00Z","event_type":"decision_recorded","id":"log_01a0db8d58c978f0943530aee5570d2b","object_refs":["task_01a0db8d588a7081a3488619c36fbe45","log_01a0db8d58c27a909584d70d185c6178"],"payload":{"action":"reopen","completion_log_id":"log_01a0db8d58c27a909584d70d185c6178","decision":"task_reopened","schema_version":"ubu.orchestrator.task_action.v1","source":{"source_id":"01a0db8d588a7081a3488619c36fbe45","source_kind":"google_calendar"},"task_status":"active","transition_applied":true},"provenance":{"authority_source":"user","created_at":"2026-09-25T10:00:00Z"}}]
```

Assertions verify active status after reopening, an increased Task version, `task_reopened`, the exact referenced completion ID, and object references naming both Task and completion. The repeat capture is unchanged and adds no third Log entry. This test observes a recent event that ended before the current planning clock, exercising the bounded lookback.

Test 5 confirmation:

```json
{"logs_unchanged":true,"newer_legacy_done_protected":true,"task_unchanged":true}
```

The test compares complete Task payload/version/status and action Log entries before and after capture, and verifies the total Log count is unchanged for an app-completed Task. It also adds a newer legacy app `task_done` fact after a Calendar completion, with equal clock timestamps, then removes colour. The latest app fact prevents reopening and no action Log is appended.

Test 6 puts the 25-hour-old event inside an explicitly wider stored Calendar horizon, so the age guard is tested on an actual observation rather than only on listing exclusion. It remains completed with identical Task and Log records. Pure checks reject exactly 24 hours and accept an end one second newer.

Tests 7 and 8 verify coloured Static and captured events complete nothing. Pure checks force an uncoloured baseline; the Static/captured guards still return no completion, reopen or diagnostic. The capture pass can separately report their unrelated owned drift.

Test 9, all three count results and the exact named drift diagnostic:

```json
{"drift":{"captured":0,"diagnostics":[{"code":"capture_owned_drift","message":"Owned Task `task_01a0db8d58957c439437934ea745e055` event `01a0db8d58957c439437934ea745e055` differs from the applied record; no Task update was made"}],"skipped":0,"unchanged":0,"updated":0},"unchanged":{"captured":0,"diagnostics":[],"skipped":0,"unchanged":1,"updated":0},"updated":{"captured":0,"diagnostics":[],"skipped":0,"unchanged":0,"updated":1}}
```

The untouched event is unchanged; a title drift leaves the Task title intact and names Task and event; a consumed colour gesture actually changes Task status. `skipped` is zero in all three cases. The diagnostic-only case is not hidden in a skip count.

Test 10 verifies the already-completed coloured event leaves Task state, Log count, mutation-envelope count, projection-result count and UniverseState identical. It still has exactly one completion entry and no second application of effects.

## Judgment calls and interpretations

| Call | Assessment |
|---|---|
| 1: Static-only category colour | Agree; step construction uses `static_anchor`, category metadata stays present. |
| 2: owned uncoloured baseline + colour means done | Agree; any colour qualifies. Pre-partition colours in the applied baseline do not masquerade as phone gestures. |
| 3: observed completion window | Agree; the recorded window comes from observation, never from the planned step. |
| 4: explicit Log source | Agree; the ordinary completion Log gains source and observed-window fields. |
| 5: most recent Calendar completion only | Agree; the query includes canonical and legacy app completion facts, ordered by append order; reopen rechecks under the action lock. |
| 6: recent reopen | Agree; exact reference rule `event.end > now - 24h`, including its strict boundary. |
| 7: Static/captured exclusion | Agree; pure detection is silent for both. Their move/resize handling remains P1B-34. |
| 8: existing capture pass | Agree; one bounded paginated observation serves capture and interaction. No new endpoint. |
| 9: fourth count bucket | Agree; unchanged, actual updates and unresolved owned drift have distinct reporting. |

No disagreement with the nine design calls. Clarifications taken from the existing code and the literal requested behavior:

- **Legacy `/done` wording:** the legacy endpoint only appends an intent Log. `/task/:id/action` with `complete` is the existing lifecycle/effects path, so Calendar completion shares that implementation. Test 3 compares against it. Legacy completion facts still count as app completions when guarding reopen. No legacy endpoint behavior was changed.
- **Narrow reopen:** no existing action performed this inverse. An internal reopen entry point uses shared Task admission and shared decision-Log recording, adding `action: "reopen"`, `decision: "task_reopened"`, and `completion_log_id`; it adds no public action enum. The Task and linked Log are both recorded, rather than a silent SQL status rewrite.
- **Effect inversion:** the requested reopen restores active status and names the undone completion. It does not attempt to invert arbitrary UniverseState effects. A later genuine completion applies ordinary effects again. Completion/admission/effect/Log writes retain the existing separate-operation model, with its existing partial-failure limitations.
- **One read plus recent history:** capture extends the observation start to the earlier of the planning start and `now - 24h`; its end stays at the planning end. Foreign capture is still restricted to the original planning horizon. Without this bounded lookback, recent past events would be invisible under a horizon starting now. No second read or unbounded history scan occurs.
- **Future-ended events:** the Quick rule has a lower age bound, not `end <= now`; the implementation preserves that literal rule. Only the most recent completion with a valid observed window and the matching Google event source is eligible.
- **Projection retention:** an accepted gesture updates the applied snapshot. Completed Calendar events remain in projection bookkeeping through regeneration and stale previews, preserving the undo gesture without a Calendar write. This does not colour active Dynamic steps, and it adds no automatic history cleanup.
- **Old applied colours:** the applied baseline must be uncoloured to detect a new completion. Old Dynamic projections require regeneration/preview/apply of the partition before using the gesture. Final review found that P1B-30 omitted absent `colorId` even on PATCH, which would leave clearing outside the request. Section F corrects `event_request`: an uncoloured PATCH explicitly sends `"colorId": null`, while INSERT and the generic event body retain omission. Test 1 asserts the exact distinction. The existing generic-body omission test keeps its expectation. No live Calendar migration was exercised; evidence establishes request encoding and recorder behavior, not a live provider round trip. Generic reconciliation repair changes the baseline, so operators should capture completion gestures before accepting them as generic drift.
- **Counts:** consumed completion/reopen signals are updates; remaining owned differences are named drift. Source recovery with an identical canonical payload is unchanged. The existing schema name is retained and OpenAPI adds the requested field.

## Known limits (verbatim)

1. **Dynamic work has no category colour on the phone.** This is the price of the completion gesture, and it is the trade Quick UbU already makes.
2. **Move and resize are not handled.** Dragging or stretching an event changes nothing in UbU. That is P1B-34, and until then a moved Static event shows as `drifted` in reconcile and is repaired back.
3. **Any colour completes.** Which colour is not interpreted, so a stray colour on a Dynamic event completes its Task.
4. **On demand.** Nothing polls, so a completion made on the phone lands only at the next capture.
5. **Reopen is bounded and one-deep.** Only the most recent completion can be undone, and only within 24 hours.
6. **A resize would fight P1B-23.** When P1B-34 adds it, note that an edited `duration_estimate` is a declaration, and P1B-23 overrides the declaration once five observations exist — so a manual resize may have no visible effect on a well-observed routine.

