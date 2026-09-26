# P1B-32 verification

Base: `718151ea30a951da866c5b3d2ba1a95309b63467`, verified clean before editing. Branch: `p1b-32-calendar-capture`. Only `ubu-orchestrator` changed. One commit per section A–F.

## Verification result

**262 tests pass, zero failures or ignored tests.** The baseline was 252; this ticket adds the required eight integration tests in `tests/calendar_capture.rs` plus two unit tests: bounded overlap/offset handling and provenance-aware ID validation/no fallback. These account for the prompt's expected increase of ten despite its eight integration scenarios.

Each section was checked with the full offline suite before committing: A 253, B 254, C 254, D 254, E 254, F 262. Final invocation:

```sh
env -i PATH="$PATH" HOME="$HOME" CARGO_NET_OFFLINE=true \
  python3 /tmp/p1b-32-results/offline-test.py \
  strace -f -qq -e trace=network -o /tmp/p1b-32-results/F-network.log \
  cargo test --locked --offline -- --nocapture
```

The wrapper installs an inherited libseccomp filter with default ALLOW and `socket` rules returning EPERM for AF_INET (2) and AF_INET6 (10), then execs its arguments. Every test run, including intermediate section checks, used this filter and a sanitized environment. Tests use only `RecordingCalendarApi`; the wire fixture decoder uses the production JSON parser without transport. The final trace contains **97 lines, zero AF_INET/AF_INET6 occurrences and zero `connect` calls**. AF_UNIX compiler IPC is local. No test performed an Internet/network connection. No live Calendar/OAuth or real Quick UbU data was used. Credentials, tokens and real Calendar/event IDs were neither read for testing nor committed, logged, returned or stored. All event fixtures are synthetic. Normal Cargo concurrency was retained; no compiler or runtime caps were introduced.

Clippy before and after used exactly:

```sh
env -i PATH="$PATH" HOME="$HOME" CARGO_NET_OFFLINE=true \
  cargo clippy --locked --offline --all-targets --message-format=json
```

Counting method: select JSON `compiler-message` records for package `ubu_orchestrator` whose message level is `warning`; deduplicate by `(code, message, primary-span file, line, column)` across targets. **10 before, 10 after, delta 0.** The complete deduplicated warning sets are identical, not only their counts. Both final commands exited successfully.

`Cargo.lock` is byte-identical to the pre-edit copy; SHA-256:

```text
4a73890bbac0991eaad68d129e5131e945336c28cf806b859369dbbf963879f9
```

`Cargo.toml` also has no diff. No dependency or pin changed. All eleven sibling repositories retain their original HEAD and clean working-tree state; the ticket's required read-only pins remain core `72b3ad8`, schemas `d12835f`, store `eeacb6e`, kernel `cf8b2ca`, GitHub adapter `963d2c8`, Quick UbU `9ccc8b8`, design `f7c4a1d`.

OpenAPI was regenerated with `cargo run --locked --offline --example generate_openapi` under the same socket-denial wrapper; it includes the capture endpoint and request/response schemas. Only new Rust files were passed to `rustfmt --edition 2021`; no repository-wide formatting was run. `git diff --check` passes.

## Verbatim evidence from the final full-suite run

The following single-line JSON values are copied unchanged from the tests' `EVIDENCE[...]` output. Task IDs are fresh synthetic admission IDs, so independent reruns allocate different IDs.

Test 1: stored Task payload, with the category also present in `tags` as required by core:

```json
{"category_tag":"personal","id":"task_01a0db5a043776d2a68f3c4eb0e84d63","occupies_capacity":true,"provenance":{"authority_source":"user","created_at":"2026-09-25T08:00:00Z","source":{"source_id":"5n0q8c9h7g4k2m1p3r6t8v0a2c","source_kind":"google_calendar"}},"static_window":{"end":"2026-09-25T09:30:00Z","start":"2026-09-25T09:00:00Z"},"status":"active","tags":["personal"],"title":"Dentist"}
```

Assertions verify exactly one Task, the title, active status, concrete static window, category, occupied capacity, full user/source provenance, one mutation envelope, and only a recorder list call. Missing/unknown capture schemas return 400 before any client call.

Test 2: the complete operation list after capture → generate → preview:

```json
[]
```

The test also verifies one planned event with the origin ID, the applied origin paired to the actual Task ID, and an apply with no Calendar write. Its source uses `+02:00` timestamps and an explicit popup reminder; canonical times and preserved reminders still yield an empty diff. Reconciliation afterwards has no conflict.

Test 3: the complete operation after editing the captured Task's static window and regenerating:

```json
{"event":{"color_id":"3","end_at":"2026-09-25T10:30:00Z","external_id":"5n0q8c9h7g4k2m1p3r6t8v0a2c","reminders_minutes":[],"start_at":"2026-09-25T10:00:00Z","summary":"Dentist","task_id":"task_01a0db5a043e7a93aa017720ed1a2646","transparent":false},"kind":"update"}
```

There is exactly one operation, `update`, against the origin rather than a create. Apply records a patch of that origin. Removing the static window returns `capture_static_required`. Completing the Task and regenerating excludes it from the plan without deleting its Calendar event. A second synthetic capacity commitment allows the existing planner to generate after the first leaves the plan.

Test 4 verifies second capture returns `captured: 0`, `updated: 1`, `skipped: 0`; identical Task payloads; version 1; one provenance source; unchanged mutation-envelope and projection-result counts. Owned phone edits count as skipped and leave the Task intact. A separate bookkeeping-loss recovery in the same test reuses the same Task/source and updates its changed title, rather than allocating another Task.

Test 5: response after creating, planning and applying an ordinary UbU Task, then capturing:

```json
{"captured":0,"diagnostics":[],"schema_version":"ubu.orchestrator.calendar_capture.v1","skipped":1,"updated":0}
```

Assertions establish `captured == 0`, `updated == 0`, `skipped == 1`; identical Tasks; unchanged mutation-envelope/projection-result counts; no source added to the ordinary Task; and precisely one `ListEvents` call. After separately removing the applied bookkeeping, the same event is unrecorded and remains uncaptured, with no canonical admission.

Test 6 verifies a transparent event captures `occupies_capacity: false`; unmapped colour produces no category and no diagnostic. With another synthetic capacity commitment, generation preserves the free event's transparency and gives an empty operation list.

Test 7 diagnostics:

```json
[{"code":"capture_all_day_unsupported","message":"list event `*` entry 1: all-day event has no dateTime"},{"code":"capture_colour_ambiguous","message":"Calendar event `5n0q8c9h7g4k2m1p3r6t8v0a2c` has a colour shared by multiple categories; no category assigned"}]
```

The recorder decodes synthetic Google list JSON using the production parser, including a timed event with default reminders and an all-day entry. Exactly one Task is captured, one entry skipped, and the captured Task has no category. A subsequent preview remains empty, preserving the ambiguous source colour instead of inventing a category.

Test 8 diagnostics:

```json
[{"code":"capture_source_removed","message":"Captured Task `task_01a0db5a043d7030adb1941318bd3e24` has no source event in the observed planning horizon; Task retained for operator review"}]
```

Reconcile makes only a list call. The Task payload and mutation-envelope count remain unchanged. Advancing the planning clock beyond the commitment yields neither missing conflicts nor source-removal diagnostics; repairing that out-of-range observation preserves its applied ownership.

## Listing and caller changes

`CalendarApi::list_events` now requires `&CalendarTimeRange`. Google passes the same validated bounds to `wire::list_request` on every page, alongside `singleEvents=true` and the optional page token. `timeMin`/`timeMax` values are percent-encoded. The existing URL test now asserts the complete encoded query.

`RecordingCalendarApi` selects seeded timed events by `start < timeMax && end > timeMin`, with timestamp offsets parsed rather than compared lexically. The new unit test exercises both excluded boundary contacts, a crossing event, equivalent offset times and invalid ranges.

Capture and reconcile resolve the planner's existing stored-Calendar or clock/configuration horizon. Reconciliation stores out-of-range applied entries separately, and repair preserves them. Observed owned events recover their Task identity from the applied record, preventing a captured source's wire placeholder ID from causing false drift. All-day diagnostics flow through the shared client diagnostic boundary.

## Judgment calls and literal interpretations

| Call | Assessment |
|---|---|
| 1: origin event ID | Agree; ordinary derivation is unchanged and invalid captured origins cannot fall back. |
| 2: record origin as applied | Agree; admission appends a bookkeeping result with no delivered operations. |
| 3: foreign only | Agree; only foreign observations reach planning/admission. Unchanged reuse counts as updated under the operator's explicit clarification. |
| 4: provenance idempotency | Agree; existing-object lookup and ordinary admission/version preconditions; no UUIDv5 machinery. |
| 5: Static capture | Agree; concrete window and edit guard against demotion. |
| 6: inverse transparency | Agree; transparent means no occupied capacity, round-trip tested. |
| 7: inverse palette | Agree; ambiguous colours diagnose, unmapped colours quietly omit category. |
| 8: report disappearance | Agree with retaining the Task; qualification: bounded absence is not global deletion, and the existing legacy reject endpoint only logs. |
| 9: bounded listing | Agree; planner-derived bounds applied by both clients and reconcile/capture. |

No design disagreement with the nine calls. The endpoint/lifecycle claim in call 8 is inaccurate in the current repository: `/task/:id/reject` does not change canonical status, and `skip` is routine-only. Documentation records that gap; this ticket adds no reject transition. The completion test verifies leaving the plan does not remove the meeting, without presenting completion as a cancellation workaround.

Other interpretations and implementation findings:

- **Foreign only versus capture twice:** the operator approved counting unchanged owned-source reuse as `updated`. It causes no second admission or projection result. Owned drift is skipped; P1B-33 interaction semantics remain deferred.
- **Pure planning versus new IDs:** `CapturedTask.task_id` is optional until admission. Existing sources carry their actual IDs. This keeps `plan_capture` deterministic and free of clock/random/store effects.
- **Category validation:** integration evidence caught the requirement that `category_tag` must occur in `tags`. Section F corrects admission to include it, preserving any existing tags.
- **Wire test updates:** the existing query assertion now includes encoded bounds. The existing strict-parser test now accepts default/omitted reminders as no explicit reminders and still rejects malformed reminder flags. This permits ordinary foreign meetings without adding dependencies. Existing diagnostic-code occurrences were searched across the whole repository including tests before moving the skip code into the shared mapper; no existing diagnostic code was removed or renamed.
- **Round-trip fields:** captured colour is preserved when no inverse category can be chosen, and recorded explicit popup reminders are preserved in projection. Other event metadata is not admitted into Tasks. Calendar default reminders are not represented; a later update uses the existing explicit projection body.
- **Calendar horizon and unsupported IDs:** absence is reported only for active captured commitments inside the observed horizon. Expanded instances must still have usable client-supplied IDs; unusable IDs are skipped rather than derived. No unbounded follow-up requests are made.
- **Existing planner behavior:** a plan is not generated when there are only non-capacity Tasks. The free-event test includes a second capacity commitment; this ticket does not alter that planner policy. With only free meetings, capture still records them and their Task capacity flags correctly.
- **Recovery:** per-Task canonical admissions and the applied snapshot are separate writes. Provenance reuse handles retries after bookkeeping loss. Inactive source Tasks are not resurrected.

## Known limits (verbatim)

1. **Capture only; the calendar is not yet an input device.** Colouring an event does not complete its Task, dragging does not move it, resizing does not change its estimate. That is P1B-33.
2. **On demand.** Nothing polls, so a meeting accepted on the phone is invisible until the operator captures.
3. **No recurrence.** `singleEvents=true` expands a recurring meeting into instances, so each occurrence captures as its own Task with no link between them and no notion of the series.
4. **No attendees, location, description or conferencing data.** A captured Task carries a title, a window, a capacity flag and possibly a category. Everything else on the event is dropped.
5. **All-day events are skipped.** They have no `dateTime`, and UbU plans concrete spans.
6. **A captured Task is Static forever.** Even if the operator would rather UbU moved it, nothing demotes it to Dynamic.
7. **Deleting a captured Task does not delete its event.** The Task leaves the plan; the meeting stays on the calendar, which is almost certainly right for a real appointment but is worth knowing.

