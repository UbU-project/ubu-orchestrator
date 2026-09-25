# P1B-29 verification

## Landing order and pin chain

Every repository was clean at preflight and matched the ticket's starting
revision. Work used branch `p1b-29-calendar-apply` in each of the six changed
repositories, with one commit per applicable lettered section. Section C has one
re-pin commit in each of its three repositories. No force push was used.

These six implementation revisions were pushed in dependency order:

| Repository | Pushed revision |
|---|---|
| `ubu-schemas` | `d12835f1f6b3519ff7c11a7fdab005f6655b33b0` |
| `ubu-core` | `72b3ad8d5734d30443ea62f721833212e6da5e25` |
| `ubu-store` | `eeacb6ef969dceaf917ff30babcb185114098a8b` |
| `ubu-planning-kernel` | `cf8b2ca41fdca0fb90e14e21369ae51d154c9330` |
| `ubu-github-adapter` | `963d2c834aa7236ca31ea32d0acc64f0e8d82341` |
| `ubu-orchestrator` | `73c01dad5486f3540a34fca065e81e5fbcc211a0` |

The orchestrator revision above is section H, the pushed implementation and
documentation parent of this section I test/verification commit. This report's
containing commit adds the seven integration tests and verification, and is the
final orchestrator branch tip reported in the final response. Its exact revision
can also be resolved with `git log -1 --format=%H -- docs/P1B-29_VERIFICATION.md`.
No dependency pin changes in section I.

Core's `schemas-ref` points exactly to the schema revision above. Store, planning
kernel and GitHub adapter each pin exactly the pushed core revision. Orchestrator
pins that core, store and adapter revision, and both planning packages pin the
pushed planning-kernel revision. Git fetches populated the existing Cargo Git
caches; Cargo updates and builds then ran offline. No path override or `[patch]`
table was introduced. Existing intra-workspace planning-crate path references
are unchanged; every cross-repository dependency remains an exact Git revision.

The user explicitly clarified that Git transport is the exception to the
no-network instruction. All other work remained offline. No new repository or
clone was created. Quick UbU, design, and every other sibling repository retained
their original HEADs and clean trees.

## Tests, Clippy and lockfiles

| Repository | Passing tests | Clippy before → after | Delta |
|---|---:|---:|---:|
| `ubu-schemas` | 1 | 0 → 0 | 0 |
| `ubu-core` | 130 | 2 → 2 | 0 |
| `ubu-store` | 100 | 0 → 0 | 0 |
| `ubu-planning-kernel` | 81 | 0 → 0 | 0 |
| `ubu-github-adapter` | 23 | 0 → 0 | 0 |
| `ubu-orchestrator` | 231 | 10 → 10 | 0 |

Schema fixture validation separately passed **76 valid and 74 invalid fixtures**,
including the new valid delete-with-payload operation and invalid unknown kind.
Core's added test round-trips a `Delete` operation, its `delete` wire value, target
and event-removal payload. Core and schemas remain `cargo fmt --all -- --check`
clean. Only newly created Rust files were formatted in orchestrator; store was
not formatted.

Tests used `cargo test --all-targets` plus `cargo test --doc` on core and
orchestrator, and their workspace equivalents on the three re-pin repositories.
Schemas used `cargo test --workspace --all-targets` and
`cargo run -p validate-fixtures`. Orchestrator section D passed **222** tests
before behavior changes; E–H passed **224**; section I passed **231**. The nine
new tests are two recording-client unit tests and exactly seven HTTP integration
tests in `tests/calendar_apply.rs`. Every lettered commit was made after its
applicable suite passed.

Clippy command on every repository's own checkout was
`cargo clippy --all-targets`, before and after in this ticket session. Counting
method: count diagnostic lines beginning `warning:`, excluding Cargo's
backtick-prefixed package summaries containing `generated N warning(s)`, including
duplicate summaries. Complete warning-message multisets match before and after
for each repository, not merely totals.

All Cargo verification commands used `CARGO_NET_OFFLINE=true`, with no compiler
concurrency or memory caps. After the desktop interruption, the completed
section D test log was recovered and confirmed to contain all 222 passing tests.
The filesystem had 11 GB free; 23 GB of rebuildable orchestrator incremental
compiler cache was removed before continuing, restoring roughly 34 GB free.
No runtime configuration was changed.

Exact lockfile checks against saved pre-ticket bytes and parsed package records:

- `ubu-schemas/Cargo.lock` and `ubu-core/Cargo.lock`: byte-identical.
- Store, planning kernel and GitHub adapter: only the `ubu_core` Git source
  changed. Its old revision was `867c7a95edcba452470604c8fbce33b2d96986c1`; the new
  source is pinned to `72b3ad8d5734d30443ea62f721833212e6da5e25`.
- Orchestrator: only the Git sources for `ubu_core`, `ubu_store`,
  `ubu_planning_core`, `ubu_planning_cpu` and `ubu_github_adapter` changed, to the
  exact revisions in the table. Every other package field and entry is identical.
- No package was added or removed. Manifests add no dependency anywhere.

`git diff --check` passed. OpenAPI was regenerated offline after persistence and
again after approve was added, and includes the preview id, policy query,
approval request, export mode, result and per-operation outcome schemas.

## Offline boundary

No application network call, credential read/handling, or Google dependency was
added anywhere. `CalendarApi` takes plain orchestrator events and ids;
`RecordingCalendarApi` stores calls and events in memory. There is no Google
client or authorization implementation. `Live` is rejected with HTTP 501
`calendar_live_export_unavailable` before any client call or delivery record.

Tests use synthetic captured Tasks, in-memory SQLite, a fixed clock, and Axum
`Router::oneshot`, without HTTP sockets or external services. Preview and apply
never call `list_events`: the persisted Calendar applied set is the only existing
side. A test inserts a newer unrelated GitHub result and confirms it is ignored.

## Test 1: permitted inserts and result records

Verbatim recording-client call log:

```json
[{"event":{"color_id":"3","end_at":"2026-09-25T09:15:00Z","external_id":"01a0d888316873709d8c773f5b778453","reminders_minutes":[],"start_at":"2026-09-25T09:00:00Z","summary":"Breakfast","task_id":"task_01a0d888316873709d8c773f5b778453","transparent":false},"kind":"insert_event"},{"event":{"color_id":"9","end_at":"2026-09-25T10:15:00Z","external_id":"01a0d888316d77729c5a2d57c9743490","reminders_minutes":[],"start_at":"2026-09-25T10:00:00Z","summary":"Work Time","task_id":"task_01a0d888316d77729c5a2d57c9743490","transparent":true},"kind":"insert_event"},{"event":{"color_id":"6","end_at":"2026-09-25T11:15:00Z","external_id":"01a0d888316e74a384a4f9ae1d71f3b0","reminders_minutes":[],"start_at":"2026-09-25T11:00:00Z","summary":"Standup","task_id":"task_01a0d888316e74a384a4f9ae1d71f3b0","transparent":false},"kind":"insert_event"}]
```

Assertions compare the entire ordered call list against exactly one insert per
preview event. All operation outcomes are applied; the persisted result's event
set equals the recorder's actual event set. Persisted and response outcomes
compare equal as core `OperationResult` values (an absent optional message and
JSON null represent the same value). Three accepted boundary Log entries record
`automation_worker`; lowering checks verify `google_calendar` targets and event
payloads.

## Test 2: unchanged second preview

Verbatim operation count:

```text
operations=0
```

The desired events remain identical, and preview makes no additional client
calls. A newer GitHub result does not change the Calendar diff. Replaying the
original preview after its base changes returns HTTP 409
`calendar_projection_conflict` without client calls. Approving the empty current
preview succeeds with zero operations and retains the applied set.

## Test 3: one update and one delete

The next synthetic day moves Breakfast from 09:00 to 09:30 and completes Standup
through the existing recorded action endpoint before regenerating. Work Time
stays unchanged. Preview proposes exactly one update followed by one delete.

Verbatim recording-client call log:

```json
[{"event":{"color_id":"3","end_at":"2026-09-25T09:45:00Z","external_id":"01a0d888316d77729c5a2cede2e5e3fb","reminders_minutes":[],"start_at":"2026-09-25T09:30:00Z","summary":"Breakfast","task_id":"task_01a0d888316d77729c5a2cede2e5e3fb","transparent":false},"kind":"patch_event"},{"external_id":"01a0d88831717680abacdddb6c814bf9","kind":"delete_event"}]
```

The lower-level delete is core `ProjectionOperationKind::Delete`; its payload is
exactly external id and summary. After apply, two events remain in the recorder
and the next preview proposes zero operations. No existing lifecycle endpoint
was changed for this test.

## Tests 4 and 5: denied authority and policy

Verbatim user-authority rejection reasons from boundary Logs:

```json
["Projection export operation calendar-create-01a0d888316873709d8c7714bf500755 used user-equivalent authority user; export is rejected by default. Export-class operations require automation_worker authority.","Projection export operation calendar-create-01a0d888316d77729c5a2d0bc5dc24c2 used user-equivalent authority user; export is rejected by default. Export-class operations require automation_worker authority.","Projection export operation calendar-create-01a0d888316e74a384a4f951306af3f9 used user-equivalent authority user; export is rejected by default. Export-class operations require automation_worker authority."]
```

Verbatim `no_external_export` rejection reasons from boundary Logs:

```json
["Compartment policy no_external_export forbids external export for projection operation calendar-create-01a0d888316873709d8c770d8afb4085.","Compartment policy no_external_export forbids external export for projection operation calendar-create-01a0d888316d77729c5a2cf56f6e10df.","Compartment policy no_external_export forbids external export for projection operation calendar-create-01a0d888316e74a384a4f946ff9149db."]
```

Both tests assert **zero recording-client calls**, failed aggregate results,
empty applied sets, and three rejected boundary Log entries. The user case
retains the caller's `user` authority in the Log and marks every operation
skipped. The policy case uses worker authority but still receives
`calendar_export_rejected` for every operation. No authority substitution bypasses
the gate.

## Tests 6 and 7: refusal and partial recovery

Live mode returns the named unavailable diagnostic with zero calls, boundary
Logs or delivery records. Missing/unknown approval schema versions also return
their existing named diagnostics without client calls.

A deterministic insert failure in a three-operation batch records three calls,
two landed events and aggregate `partial`. The next preview proposes exactly the
failed event. An entirely failed subsequent attempt does not erase the partial
snapshot. With a healthy recorder seeded from the two landed events, the next
apply inserts only the remaining event; the following preview is empty. This
also exercises insertion-order selection when all results share the fixed clock.
The recording-client unit tests independently verify seeded CRUD call order and
atomic failures for insert, patch and delete, with no hidden retries.

## Existing tests and re-pin boundaries

Section C required **no production source changes**, new match arms, or test
updates in store, planning kernel or GitHub adapter. Their diffs contain only
`Cargo.toml` and `Cargo.lock` pin changes, and all suites pass.

The user approved updating P1B-28's existing preview test. It now compares response
content excluding unique preview ids, asserts both preview records exist, and
snapshots every other SQLite table to prove canonical and unrelated state stays
unchanged. The test was renamed to
`repeated_preview_preserves_content_and_only_persists_previews`. Its mapping,
ordering and unmappable-id checks remain. No other pre-existing test changed.

Before relocating/reusing diagnostic handling, a whole-repository `rg` search
included `tests/` for `calendar_event_id_unmappable`, `unknown_schema_version` and
`missing_schema_version`. Those existing codes are unchanged. The new apply
codes describe unavailable Live export, gate rejection, client failure and a
changed applied-set base.

## Judgment calls and interpretations

No disagreement with the eight judgment calls. Implementation interpretations:

- “Most recent successful result” includes a partial batch's successfully landed
  operations. Excluding partial snapshots would contradict judgment call 5 and
  test 7 by re-proposing operations that already landed. Entirely failed batches
  are excluded from the existing-side query.
- Calendar results are filtered by their schema version and selected by durable
  insertion order, so other projection surfaces and tied/backwards timestamps
  do not replace the wrong applied set.
- Preview's new `preview_id` is necessary for approving its persisted snapshot.
  The user-approved existing test therefore compares semantic content rather
  than random ids. Projection records are permitted writes; canonical state is
  still unchanged by preview.
- The optional `no_external_export` preview query mirrors the existing GitHub
  policy input. A stored resolved policy is passed to the unchanged core gate.
  Null/unresolved policy receives no permit. This adds no new policy resolver or
  worker authority model.
- Valid processed approvals return HTTP 200 with applied/partial/failed outcomes;
  Live refusal is HTTP 501. Export mode uses lowercase `mock` / `live`, and
  authority and mode are required rather than silently defaulted.
- A successful no-op batch records the existing set; a fully deleted set is an
  explicit empty successful snapshot. Failure of an update/delete retains the
  previous event in the applied set.
- A shared controller lock serializes preview/apply. A preview based on a changed
  applied set returns 409 before client calls, requiring a fresh preview.
- Recording mode is seeded from persisted belief when no test client is injected;
  it never consults an external calendar. The client seam can be extracted later.
- The network prohibition excludes Git by explicit user clarification. No path
  override was used for the pin chain; existing internal workspace paths remain.

## Known limits

1. **Nothing reaches Google.** `Live` is refused. The whole path is exercised against `RecordingCalendarApi`, so the trait's shape is proven and its Google implementation is not.
2. **The applied set is UbU's belief, not the calendar's state.** If someone edits or deletes an event in Google, UbU will not notice and the next preview will propose nothing. Reconciliation is P1B-30.
3. **No retry and no backoff.** A failed operation is recorded as failed and re-proposed by the next preview. There is no queue and no automatic second attempt.
4. **One calendar, not selected.** Which calendar receives the projection is still unmodelled; it belongs with the client and its credentials.
5. **The boundary Log records adjudication, not delivery.** A permitted operation that then fails in the client is visible in the result payload, not in the boundary Log entry, which was written when the gate decided.
