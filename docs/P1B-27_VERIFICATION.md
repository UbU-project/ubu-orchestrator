# P1B-27 verification

## Scope and checks

Implemented sections A–E on `p1b-27-task-capture`, starting from orchestrator
`a39dcc9d218e9e0981fa7cdee223cec0b8a40ca9`. The initial tree was clean.
Only `ubu-orchestrator` changed. All eleven sibling repositories retained their
initial HEADs and clean trees. No dependency was added, no pin moved, and both
`Cargo.toml` and `Cargo.lock` are byte-identical to their pre-ticket copies.
All fixtures are synthetic; no real Quick UbU data was used or committed.

`cargo test --all-targets` and `cargo test --doc` passed before each lettered
commit. Sections A–D each passed 204 tests. Section E passed **211 tests**, with
zero failures or ignored tests: the original 204 plus exactly seven new tests in
`tests/task_capture.rs`. The doc-test target has zero tests. The targeted command
`cargo test --test task_capture -- --nocapture` passed all seven and produced the
verbatim evidence below. No pre-existing test was modified.

`cargo clippy --all-targets` exited successfully before and after this ticket in
the same development session: **10 diagnostic warning headers before, 10 after,
delta 0**. Counting method: count lines beginning `warning:`, excluding Cargo's
backtick-prefixed package summaries containing `generated N warning(s)` (including
duplicate summaries). The full warning-message multisets also match. There were
no new Clippy diagnostics. Normal Cargo concurrency was used without compiler or
memory caps.

`cargo run --example generate_openapi` regenerated the committed artifact. It
contains POST `/task`, PATCH `/task/{task_id}`, the two request schemas, and the
write response schema; `expected_version` is required on edits. Only newly
created Rust files were formatted with `rustfmt --edition 2021`; repository-wide
formatting was not run. `git diff --check` passed.

## Captured payload and planning (test 1)

Verbatim stored payload:

```json
{"duration_estimate":{"seconds":900,"type":"fixed"},"id":"task_01a0d69bf6837c6192b79d9c10579946","provenance":{"authority_source":"user","created_at":"2026-09-24T09:00:00Z"},"status":"active","title":"Call the plumber"}
```

The test compares the entire stored JSON with the expected Task, verifies the
`user-capture` compartment and active status, and explicitly asserts that
`provenance.source` is absent. A subsequent POST `/planning/generate` returns an
OK plan containing this Task; resolving its scheduled id confirms the title
`Call the plumber`.

## Conflict and unchanged storage (test 4)

Verbatim HTTP 409 body from the second edit with stale expected version 1:

```json
{"diagnostics":[{"code":"version_conflict","message":"Task `task_01a0d69bf6827261afd969398666fc7e` expected version 1, current version 2"}],"error":"Task `task_01a0d69bf6827261afd969398666fc7e` expected version 1, current version 2"}
```

The first edit stored title `Kept` at version 2. The stale edit attempted title
`Must not write`. After the 409, assertions compare `payload_json` byte-for-byte,
version, and updated timestamp against the record before the request: unchanged.
The same test deterministically submits a stale admission envelope to the actual
store, receives `StoreError::PreconditionFailed`, verifies its HTTP 409 mapping,
and again verifies unchanged storage. This exercises the real precondition guard
without relying on race timing.

Section B changed **only** `StoreError::PreconditionFailed` from HTTP 500 to 409.
The remaining store-error match arm is unchanged; representative
`MissingTargetPrecondition` and `DuplicateOccurrenceKey` errors still assert HTTP
500. Existing diagnostic strings and response-body handling were not changed.
Whole-repository diagnostic searches included `tests/` before the relevant edits.

## Routine occurrence rejection (test 6)

Verbatim HTTP 400 body after importing a synthetic routine and materializing it
through POST `/planning/generate`:

```json
{"diagnostics":[{"code":"routine_occurrence_not_editable","message":"Task `task_01a0d69bf69573f19dfbd5da2dd5beec` is a routine occurrence; edit routine `obj_01a0d69bf6857170b601c6e5295e9583` instead"}],"error":"Task `task_01a0d69bf69573f19dfbd5da2dd5beec` is a routine occurrence; edit routine `obj_01a0d69bf6857170b601c6e5295e9583` instead"}
```

The diagnostic names the routine to edit. Both the correct version and an
incorrect version return `routine_occurrence_not_editable`; the occurrence's
stored payload stays unchanged. The occurrence check therefore precedes the
version comparison as required.

## Two successive imports (test 7)

The synthetic snapshot includes a routine and a manual Quick UbU Task, ensuring
the importer performs actual writes in both rounds. The second round changes
only the imported Task's title.

Round 1 response counts and isolation lists, verbatim:

```json
{"diverged":[],"preferences":{"created":0,"unchanged":0,"updated":0},"routines":{"created":1,"unchanged":0,"updated":0},"stale":[],"tasks":{"created":1,"unchanged":0,"updated":0}}
```

Round 2 response counts and isolation lists, verbatim:

```json
{"diverged":[],"preferences":{"created":0,"unchanged":0,"updated":0},"routines":{"created":0,"unchanged":1,"updated":0},"stale":[],"tasks":{"created":0,"unchanged":0,"updated":1}}
```

After **each** round, the captured Task remains active at version 1 with title
`Call the plumber`, byte-identical payload, and no entry in `stale` or `diverged`.
The first import creates one routine and one imported Task; the second keeps the
routine unchanged and updates only the imported Task. Captured user provenance
has no source and does not match the importer's `source_kind = 'quick_ubu'` scope.

## Other assertions and interpretation

The remaining tests verify title-only capture, missing/empty/null titles,
server-owned and unknown field rejection, schema diagnostics, a successful edit
to version 2, preserved server metadata, HTTP 404 `unknown_task`, required
`expected_version`, null removal of a due date, and rejection of a zero-duration
Task with the core Task validator's own error and no admission.

No disagreement with the seven judgment calls; all seven were implemented.
Literal readings for unspecified details:

- Unknown, malformed, or non-Task ids return HTTP 404 `unknown_task`.
- Routine occurrence rejection and Task validation failures return HTTP 400.
  Existing global `Core` error mapping is unchanged; validation errors at this
  new input boundary are reported as bad requests with the core error text.
- A null title is missing; an empty string is empty. Titles are not trimmed or
  normalized beyond the pinned core validator's behavior.
- Explicit null removal applies to PATCH. Capture preserves supplied values and
  lets Task deserialization/validation decide whether they are valid.
- Every key outside the shared list is rejected, even if its value is null.
  Schema version and edit expected version are request metadata, not Task fields.
- `schema_version` uses `Option<String>` to preserve the existing named missing
  diagnostic pattern. Missing `expected_version` uses Axum's HTTP 422 extraction
  rejection. No extra diagnostic code was invented for that case.
- No-op valid edits still admit the next version, as specified. Existing status,
  provenance, creation time and compartment are preserved.

The five known limits are recorded verbatim below. The assertion that a racing
write is unreachable on a single-Device instance was not established by these
checks: concurrent requests can still justify the store guard. The deterministic
precondition test verifies that guard regardless of deployment concurrency.

## Known limits

1. **The version check is advisory.** It reads the current version, then admits. A write landing between the two is caught by the store's own precondition, which is why §B maps it to `409`. On a single-Device desktop instance the window is not reachable in practice; it is named because Phase 2 sync makes it real.
2. **No capture surface for routines or Objectives.** They still come from `routine.json` through Quick UbU, so `establishes`, `requires` and recurrence remain unauthorable in mainline.
3. **No delete.** A captured Task can be rejected, snoozed or completed through the existing action endpoints, but there is no way to remove one that should never have existed.
4. **No UI.** This is an HTTP surface. `ubu-ui` has no capture affordance, so until it does, capture during the day means an HTTP call.
5. **`blocked_by` and `objective_id` are editable but unvalidated as references.** Nothing checks that the named Task or Objective exists; a dangling `blocked_by` is handled downstream by the existing dependency machinery, not rejected here.

