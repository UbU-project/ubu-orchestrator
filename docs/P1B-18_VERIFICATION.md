# P1B-18 verification

All seven implementation repositories use branch `p1b-18-routine-import`.
All 12 sibling working trees were clean before work and after generated-config
cleanup, before adding this verification document. `ubu-design`, `ubu-devshell`,
and `ubu-ui` have no changes. No dependency was added. All fixtures are synthetic;
no real Quick UbU data was read or committed.

## Revisions and dependency order

Quick UbU was pushed first, followed by schemas, core, store, adapter, and kernel.
The orchestrator pins all five mainline dependencies to those pushed revisions.
Section K commits this verification document; section L records final verification
in an empty orchestrator commit, after which the final orchestrator rev is pushed
and reported in the ticket completion response.

| Repository | Pushed implementation revision |
|---|---|
| quick-ubu | `a584bca94c887fd9e11b7870e206ef8535c1edc7` |
| ubu-schemas | `2eae0bd7f2274a28c56f043fc7a8875fc981abd3` |
| ubu-core | `cc2087f7e9bb54029a60b34916d4ac6d291604f0` |
| ubu-store | `ddee9621525f77fac29c28dc513cb2bf4b463419` |
| ubu-github-adapter | `2ffb7f47e2d2806828c29badfa9c8869a0641a68` |
| ubu-planning-kernel | `24508908147ef2bebc63b5e71d695b9d9c4be425` |
| ubu-orchestrator | `a9bcd1ac62946fd11a7f6241868f456527ae21e1` (tested I/J implementation; final K/L revision reported on completion) |

## Test results

- Quick UbU: 443 tests passed. The new snapshot test covers generated, orphaned,
  Calendar capture, and manual origins, store equality, version 1, and legacy input.
- Schemas: 75 valid and 70 invalid fixtures validated.
- Core: 128 tests passed, including new fixture round-trips and named validators.
- Store: 100 tests passed, including piecemeal validation, all four Objective
  cross-field rules, occurrence uniqueness, same-Task update, absent keys, and the
  independent database index backstop.
- Adapter: 23 tests passed; only its core pin and lockfile source changed.
- Kernel: 56 tests passed. Every kernel golden matched; kernel source, existing
  tests, and all fixtures are byte-for-byte unchanged from `8e22b57`.
- Orchestrator: 138 tests passed with committed dependencies, then again with
  local patches. Existing suites passed. Import tests cover exact counts,
  no-write retries, independent version bumps, stale objects, mainline completion,
  clock advances, disabled Preferences, dry runs, malformed inputs, the one-time
  parent heuristic, and concurrent retries.

The initial patched orchestrator verification was interrupted when the operator
reported an out-of-memory kill of Codex. The patched suite was rerun successfully
with `CARGO_BUILD_JOBS=1 cargo test -j 1 -- --test-threads=1`. Subsequent Rust builds
used one compiler job.

A separate real CLI smoke test exported an empty synthetic SQLite store, copied
its serialized Store into a synthetic legacy JSON file, and exported that file
with `--from-json`. Both snapshots were equal; version was 1. The legacy command
did not open its unused SQLite path. All smoke-test data remained in `/tmp`.

OpenAPI was regenerated with `cargo run --example generate_openapi` (the successful
retry used `--offline --locked` after the new git revisions were cached).

### quick-ubu: verbatim pass/fail tail

```text
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests gcal

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ollama_planner

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

### ubu-schemas: verbatim pass/fail tail

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.03s
     Running `target/debug/validate-fixtures`
validated 75 valid fixtures
validated 70 invalid fixtures
```

### ubu-core: verbatim pass/fail tail

```text
     Running tests/timestamp_validation.rs (target/debug/deps/timestamp_validation-1ff7ac2ed8333aaf)

running 2 tests
test accepts_rfc3339_timestamps_with_offsets ... ok
test rejects_naive_timestamps ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

### ubu-store: verbatim pass/fail tail

```text
test admits_universe_state_with_envelope_and_provenance_authority_source ... ok
test universe_requires_version_target_and_checks_read_preconditions ... ok
test universe_payload_or_authority_change_conflicts_on_replay ... ok
test universe_ledger_failure_rolls_back_updated_object ... ok
test universe_envelope_update_replay_and_stale_precondition ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

   Doc-tests ubu_store

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

### ubu-github-adapter: verbatim pass/fail tail

```text

     Running tests/reconciliation.rs (target/debug/deps/reconciliation-2d69e01f6c38092f)

running 1 test
test reconciles_matching_projection_result ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_github_adapter

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

### ubu-planning-kernel: verbatim pass/fail tail

```text
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_planning_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_planning_cpu

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

### ubu-orchestrator: verbatim pass/fail tail

```text
test complete_records_decision_log_and_transitions_task ... ok
test complete_with_effects_but_no_universe_state_surfaces_diagnostic ... ok
test complete_without_effects_leaves_universe_state_unchanged ... ok
test override_records_user_override_without_transition ... ok
test record_action_requires_known_schema_version ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s

   Doc-tests ubu_orchestrator

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

## Section L: patched and unpatched dependency verification

With the generated local config, `cargo tree -i ubu_core` showed one path source:

```text
ubu_core v0.1.0 (/home/sean/ubu-phase1b/ubu-core)
├── ubu_github_adapter v0.1.0 (/home/sean/ubu-phase1b/ubu-github-adapter)
│   └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_core v0.1.0 (/home/sean/ubu-phase1b/ubu-planning-kernel/crates/ubu-planning-core)
│   ├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
│   └── ubu_planning_cpu v0.1.0 (/home/sean/ubu-phase1b/ubu-planning-kernel/crates/ubu-planning-cpu)
│       └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_cpu v0.1.0 (/home/sean/ubu-phase1b/ubu-planning-kernel/crates/ubu-planning-cpu) (*)
└── ubu_store v0.1.0 (/home/sean/ubu-phase1b/ubu-store)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
```

Moved `.cargo/config.toml` aside, restored the committed git-source lockfile,
and ran `CARGO_BUILD_JOBS=1 cargo build --offline --locked -j 1` successfully.
The config was restored and then removed as required for final cleanup.
The unpatched `cargo tree --offline --locked -i ubu_core` has one committed core
source at `cc2087f7e9bb54029a60b34916d4ac6d291604f0`:

```text
ubu_core v0.1.0 (https://github.com/UbU-project/ubu-core?rev=cc2087f7e9bb54029a60b34916d4ac6d291604f0#cc2087f7)
├── ubu_github_adapter v0.1.0 (https://github.com/UbU-project/ubu-github-adapter?rev=2ffb7f47e2d2806828c29badfa9c8869a0641a68#2ffb7f47)
│   └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_core v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=24508908147ef2bebc63b5e71d695b9d9c4be425#24508908)
│   ├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
│   └── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=24508908147ef2bebc63b5e71d695b9d9c4be425#24508908)
│       └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=24508908147ef2bebc63b5e71d695b9d9c4be425#24508908) (*)
└── ubu_store v0.1.0 (https://github.com/UbU-project/ubu-store?rev=ddee9621525f77fac29c28dc513cb2bf4b463419#ddee9621)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
```

Verbatim locked-build tail:

```text
   Compiling ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.56s
```

A lockfile audit confirmed no added packages and no dependency changes beyond
the requested git source pins. Core's dependency manifest is unchanged. Generated
configs were removed from all mainline consumers, and every sibling tree was
checked clean before this document was added.

## Verbatim first import response

The synthetic fixture creates five routine Objectives, five one-off Tasks, and
two Task Preferences. It reports one legacy Objective not imported and 15 item
or field skips. No occurrence is created or planned.

```json
{
  "diverged": [],
  "dry_run": false,
  "objectives_not_imported": 1,
  "preferences": {
    "created": 2,
    "unchanged": 0,
    "updated": 0
  },
  "routines": {
    "created": 5,
    "unchanged": 0,
    "updated": 0
  },
  "schema_version": "quick-ubu-import/1",
  "skipped": [
    {
      "kind": "routine",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000001",
      "reason": "negative_reminder"
    },
    {
      "kind": "routine",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000002",
      "reason": "after_reference_missing"
    },
    {
      "kind": "routine",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000002",
      "reason": "after_self_reference"
    },
    {
      "kind": "routine",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000002",
      "reason": "negative_after_offset"
    },
    {
      "kind": "task",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000012",
      "reason": "past_static_window"
    },
    {
      "kind": "task",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000013",
      "reason": "completed"
    },
    {
      "kind": "task",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000014",
      "reason": "task_after_unsupported"
    },
    {
      "kind": "task",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000016",
      "reason": "routine_occurrence"
    },
    {
      "kind": "task",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000017",
      "reason": "orphaned_routine_occurrence"
    },
    {
      "kind": "task",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000018",
      "reason": "partial_allowed_range"
    },
    {
      "kind": "task",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000019",
      "reason": "invalid_allowed_range"
    },
    {
      "kind": "task",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000020",
      "reason": "invalid: Task tags must not contain empty strings"
    },
    {
      "kind": "task",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000014",
      "reason": "dependency_not_imported"
    },
    {
      "kind": "preference",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000033|00000000-0000-4000-8000-000000000032",
      "reason": "non_singleton_bundle"
    },
    {
      "kind": "preference",
      "quick_ubu_id": "00000000-0000-4000-8000-000000000034|00000000-0000-4000-8000-000000000031",
      "reason": "task_not_imported"
    }
  ],
  "stale": [],
  "tasks": {
    "created": 5,
    "unchanged": 0,
    "updated": 0
  }
}
```

## Deviations and literal-reading assumptions

No functional scope expansion or omitted ticket work.

- Setup retry (`P1B-18_prompt.md:15–27`): the initial update script overlapped this
  ticket's own schema edits and refused that dirty tree. Those edits were stashed,
  temporary patch effects were removed, and the complete update/setup was rerun
  successfully with clean trees before restoring the edits. No pre-existing work
  was changed and no remote update changed any prerequisite revision.
- Snapshot precision (`src/services/quick_ubu_import.rs:190`, `:279`, `:384`):
  chrono tuples with nonzero nanoseconds cannot be represented losslessly in the
  specified whole-second model. Such routine/Task items are skipped with `invalid:`;
  fractional after entries are individually dropped with `invalid:`. No rounding
  or new duration representation was introduced.
- Response version (`src/services/quick_ubu_import.rs:320`): the ticket requires
  a response schema_version but does not prescribe its value. The value is
  `quick-ubu-import/1`; the input snapshot version remains numeric 1.
- Duplicate Preference identity (`src/services/quick_ubu_import.rs:457`): the
  ordered bundle pair identifies a single object. The first source entry wins;
  later entries are reported as `invalid: duplicate preference source identity`.
- Commit policy (`P1B-18_prompt.md:42`, `:405`, `:409`): K owns the verification
  document in the orchestrator. L has no source edits and is recorded as an empty
  verification commit in that same repo, preserving one commit per lettered section.
- Build recovery (`P1B-18_prompt.md:409`): after the operator-reported memory
  interruption, remaining Rust work ran serially. The interrupted patched run
  was replaced by a complete passing run; no check was waived.

## Repository trees

### quick-ubu: `tree -I target`

```text
.
├── Cargo.lock
├── Cargo.toml
├── cli
│   ├── Cargo.toml
│   ├── src
│   │   ├── batch.rs
│   │   ├── batch_tests.rs
│   │   ├── clarify.rs
│   │   ├── clarify_session_tests.rs
│   │   ├── clarify_tests.rs
│   │   ├── decompose.rs
│   │   ├── decompose_rewire_tests.rs
│   │   ├── decompose_suggest_tests.rs
│   │   ├── decompose_tests.rs
│   │   ├── log_query_tests.rs
│   │   ├── logic.rs
│   │   ├── main.rs
│   │   ├── persist.rs
│   │   ├── sqlite.rs
│   │   ├── test_support.rs
│   │   └── watch.rs
│   └── tests
│       └── commands.rs
├── core
│   ├── Cargo.toml
│   └── src
│       ├── decision.rs
│       ├── history.rs
│       ├── lib.rs
│       ├── plan.rs
│       ├── planning.rs
│       ├── precompute.rs
│       ├── project.rs
│       ├── provisional.rs
│       ├── reconcile.rs
│       ├── report.rs
│       ├── routine.rs
│       ├── store.rs
│       ├── tests.rs
│       └── types.rs
├── docs
│   ├── SPEC.md
│   ├── acceptance-copy.md
│   ├── calendar-import.md
│   ├── example-routine.json
│   ├── ollama-streaming.md
│   └── review-order.md
├── gcal
│   ├── Cargo.toml
│   └── src
│       ├── import_tests.rs
│       └── lib.rs
├── ollama-planner
│   ├── Cargo.toml
│   └── src
│       └── lib.rs
└── tests
    ├── quick_ubu_test_copy.py
    └── test_quick_ubu_test_copy.py

12 directories, 48 files
```

### ubu-schemas: `tree -I target`

```text
.
├── CHANGELOG.md
├── CODEGEN.md
├── CONTRACT.md
├── CONTRIBUTING.md
├── Cargo.lock
├── Cargo.toml
├── LICENSE
├── PLANNING_KERNEL_CONTRACT.md
├── README.md
├── SECURITY.md
├── clippy.toml
├── fixtures
│   ├── invalid
│   │   ├── api
│   │   │   ├── human-complete-plan-quality
│   │   │   │   └── character-judgment.json
│   │   │   └── risk-report
│   │   │       └── legacy-items.json
│   │   ├── artifact
│   │   ├── common
│   │   │   ├── authority-source
│   │   │   │   ├── object-user-value.json
│   │   │   │   └── unknown-value.json
│   │   │   ├── id
│   │   │   │   ├── container-wrong-prefix.json
│   │   │   │   ├── external-event-wrong-prefix.json
│   │   │   │   ├── hyphenated-suffix.json
│   │   │   │   ├── identity-wrong-prefix.json
│   │   │   │   ├── preference-wrong-prefix.json
│   │   │   │   ├── relationship-wrong-prefix.json
│   │   │   │   ├── setting-wrong-prefix.json
│   │   │   │   ├── universe-state-wrong-prefix.json
│   │   │   │   ├── uppercase-suffix.json
│   │   │   │   ├── wrong-length.json
│   │   │   │   ├── wrong-prefix.json
│   │   │   │   ├── wrong-variant-bit.json
│   │   │   │   └── wrong-version-bit.json
│   │   │   ├── policy-summary
│   │   │   │   └── unknown-member.json
│   │   │   ├── provenance
│   │   │   │   └── github-event-authority.json
│   │   │   └── timestamp
│   │   │       └── naive.json
│   │   ├── core
│   │   │   ├── advisory-candidate
│   │   │   │   ├── missing-required-field.json
│   │   │   │   ├── out-of-range-confidence.json
│   │   │   │   └── unknown-lifecycle-state.json
│   │   │   ├── device-registration
│   │   │   │   ├── missing-required-field.json
│   │   │   │   ├── non-identity-prefix.json
│   │   │   │   └── zone-id-array.json
│   │   │   ├── external-reference
│   │   │   │   └── bad-url.json
│   │   │   ├── log-entry
│   │   │   │   ├── compartment-boundary-missing-authority-source.json
│   │   │   │   ├── compartment-boundary-missing-provenance.json
│   │   │   │   └── empty-message.json
│   │   │   ├── objective
│   │   │   │   ├── missing-status.json
│   │   │   │   ├── planned-without-range.json
│   │   │   │   ├── recurrence-on-one-time.json
│   │   │   │   ├── routine-with-priority.json
│   │   │   │   ├── template-without-recurrence.json
│   │   │   │   ├── unknown-rule-kind.json
│   │   │   │   └── weekly-without-weekdays.json
│   │   │   ├── precondition
│   │   │   │   └── unknown-predicate.json
│   │   │   ├── preference
│   │   │   │   ├── incomplete-pair.json
│   │   │   │   ├── mixed-pair.json
│   │   │   │   └── unknown-order.json
│   │   │   ├── snapshot
│   │   │   │   └── stale-tolerance-fields.json
│   │   │   ├── task
│   │   │   │   ├── active-with-moot-reason.json
│   │   │   │   ├── allowed-time-range-missing-latest-finish.json
│   │   │   │   ├── duplicate-correlation-group.json
│   │   │   │   ├── duplicate-tags.json
│   │   │   │   ├── effects-unknown-field.json
│   │   │   │   ├── invalid-duration-order.json
│   │   │   │   ├── invalid-duration-ordering.json
│   │   │   │   ├── moot-without-reason.json
│   │   │   │   ├── static-window-missing-end.json
│   │   │   │   ├── static-with-allowed-time-range.json
│   │   │   │   └── unknown-field.json
│   │   │   └── universe-state-mutation
│   │   │       └── missing-payload.json
│   │   ├── github
│   │   │   └── github-issue-source
│   │   │       └── bad-number.json
│   │   ├── projection
│   │   │   ├── projection-approval
│   │   │   │   └── bad-approved.json
│   │   │   ├── projection-preview
│   │   │   │   └── empty-operations-item.json
│   │   │   └── projection-result
│   │   │       └── bad-status.json
│   │   ├── store
│   │   │   ├── admitted-object
│   │   │   │   └── missing-provenance-authority-source.json
│   │   │   ├── mutation-envelope
│   │   │   │   ├── authoritative-execution-context.json
│   │   │   │   ├── leading-zero-version.json
│   │   │   │   ├── malformed-version-reference.json
│   │   │   │   └── missing-required-field.json
│   │   │   └── recalculation-trigger
│   │   │       ├── missing-trigger-type.json
│   │   │       └── old-reason.json
│   │   └── worker
│   │       ├── gpu-advisory-request
│   │       │   └── empty-workload.json
│   │       ├── gpu-advisory-response
│   │       │   └── bad-recommendation.json
│   │       ├── local-advisory-result
│   │       │   ├── timeout-with-candidates.json
│   │       │   └── unknown-status.json
│   │       └── local-advisory-submission
│   │           └── empty-capability-set.json
│   └── valid
│       ├── api
│       │   ├── human-complete-plan-quality
│       │   │   └── all-signals.json
│       │   └── risk-report
│       │       └── categorized-findings.json
│       ├── artifact
│       ├── common
│       │   ├── authority-source
│       │   │   └── user.json
│       │   ├── id
│       │   │   ├── container-id.json
│       │   │   ├── external-event-id.json
│       │   │   ├── identity-id.json
│       │   │   ├── preference-id.json
│       │   │   ├── relationship-id.json
│       │   │   ├── setting-id.json
│       │   │   ├── task-id.json
│       │   │   └── universe-state-id.json
│       │   ├── policy-summary
│       │   │   └── guardrail-members.json
│       │   ├── provenance
│       │   │   ├── system-github-source-ref.json
│       │   │   └── user.json
│       │   └── timestamp
│       │       └── offset.json
│       ├── core
│       │   ├── advisory-candidate
│       │   │   ├── deferred.json
│       │   │   ├── proposed-tag.json
│       │   │   ├── redacted-payload.json
│       │   │   └── resurfaced.json
│       │   ├── device-registration
│       │   │   ├── phase1b-single-device.json
│       │   │   └── revoked.json
│       │   ├── external-reference
│       │   │   └── basic.json
│       │   ├── log-entry
│       │   │   ├── basic.json
│       │   │   └── compartment-boundary-decided.json
│       │   ├── objective
│       │   │   ├── basic.json
│       │   │   ├── routine-planned.json
│       │   │   └── routine-static.json
│       │   ├── precondition
│       │   │   └── tree.json
│       │   ├── preference
│       │   │   ├── objective-pair.json
│       │   │   └── task-pair.json
│       │   ├── setting
│       │   │   └── basic.json
│       │   ├── snapshot
│       │   │   ├── bootstrap-defaults.json
│       │   │   └── live-observation.json
│       │   ├── suppression-record
│       │   │   └── rejected-tag.json
│       │   ├── task
│       │   │   ├── basic.json
│       │   │   ├── default-duration.json
│       │   │   ├── fixed-duration.json
│       │   │   ├── moot.json
│       │   │   ├── non-capacity.json
│       │   │   ├── routine-occurrence.json
│       │   │   ├── shifted-lognormal-duration.json
│       │   │   ├── static-task.json
│       │   │   ├── tagged.json
│       │   │   ├── with-allowed-time-range.json
│       │   │   ├── with-category-tag.json
│       │   │   ├── with-drift-fields.json
│       │   │   ├── with-effects.json
│       │   │   └── with-preconditions.json
│       │   ├── universe-state
│       │   │   └── populated.json
│       │   └── universe-state-mutation
│       │       ├── add-membership.json
│       │       ├── append-event-marker.json
│       │       ├── clear-fact.json
│       │       ├── decrement-numeric.json
│       │       ├── increment-numeric.json
│       │       ├── remove-membership.json
│       │       └── set-fact.json
│       ├── github
│       │   └── github-issue-source
│       │       └── basic.json
│       ├── planning
│       │   ├── affect-profile
│       │   │   └── bootstrap-default-review-priors.json
│       │   ├── plan
│       │   │   └── superseding-timed.json
│       │   └── plan-step
│       │       └── timed.json
│       ├── projection
│       │   ├── projection-approval
│       │   │   └── basic.json
│       │   ├── projection-preview
│       │   │   └── basic.json
│       │   └── projection-result
│       │       └── basic.json
│       ├── store
│       │   ├── admitted-object
│       │   │   └── basic.json
│       │   ├── mutation-envelope
│       │   │   ├── basic.json
│       │   │   ├── create-absence-precondition.json
│       │   │   ├── overnight-advisory.json
│       │   │   └── with-policy-versions.json
│       │   └── recalculation-trigger
│       │       └── basic.json
│       └── worker
│           ├── gpu-advisory-request
│           │   └── basic.json
│           ├── gpu-advisory-response
│           │   └── basic.json
│           ├── local-advisory-result
│           │   ├── ok.json
│           │   ├── partial.json
│           │   └── timeout.json
│           └── local-advisory-submission
│               └── tag-proposal.json
├── generated
│   └── typescript
│       └── README.md
├── package-lock.json
├── package.json
├── rust-toolchain.toml
├── schemas
│   ├── api
│   │   ├── bootstrap-request.schema.json
│   │   ├── bootstrap-response.schema.json
│   │   ├── human-complete-plan-quality.schema.json
│   │   ├── human-complete-report.schema.json
│   │   ├── next-action-response.schema.json
│   │   ├── risk-report.schema.json
│   │   └── user-action-request.schema.json
│   ├── artifact
│   │   ├── artifact-manifest.schema.json
│   │   ├── claim-register.schema.json
│   │   ├── evidence-index.schema.json
│   │   ├── export-review.schema.json
│   │   └── publication-plan.schema.json
│   ├── common
│   │   ├── authority-source.schema.json
│   │   ├── compartment-label.schema.json
│   │   ├── duration.schema.json
│   │   ├── error.schema.json
│   │   ├── id-registry.schema.json
│   │   ├── id.schema.json
│   │   ├── money.schema.json
│   │   ├── object-ref.schema.json
│   │   ├── policy-summary.schema.json
│   │   ├── provenance.schema.json
│   │   ├── source-ref.schema.json
│   │   └── timestamp.schema.json
│   ├── core
│   │   ├── advisory-candidate.schema.json
│   │   ├── automation-worker.schema.json
│   │   ├── compartment.schema.json
│   │   ├── container.schema.json
│   │   ├── device-registration.schema.json
│   │   ├── external-event.schema.json
│   │   ├── external-reference.schema.json
│   │   ├── identity.schema.json
│   │   ├── log-entry.schema.json
│   │   ├── moot-reason-code.schema.json
│   │   ├── objective.schema.json
│   │   ├── precondition.schema.json
│   │   ├── preference.schema.json
│   │   ├── relationship.schema.json
│   │   ├── setting.schema.json
│   │   ├── snapshot.schema.json
│   │   ├── suppression-record.schema.json
│   │   ├── task-status.schema.json
│   │   ├── task.schema.json
│   │   ├── universe-state-mutation.schema.json
│   │   ├── universe-state.schema.json
│   │   └── work-item.schema.json
│   ├── github
│   │   ├── github-ci-event-source.schema.json
│   │   ├── github-comment-source.schema.json
│   │   ├── github-import-batch.schema.json
│   │   ├── github-issue-source.schema.json
│   │   ├── github-label-source.schema.json
│   │   ├── github-milestone-source.schema.json
│   │   ├── github-pr-source.schema.json
│   │   ├── github-repository-source.schema.json
│   │   └── github-review-source.schema.json
│   ├── planning
│   │   ├── affect-profile.schema.json
│   │   ├── calendar.schema.json
│   │   ├── explanation-fragment.schema.json
│   │   ├── plan-step.schema.json
│   │   └── plan.schema.json
│   ├── projection
│   │   ├── github-comment-write.schema.json
│   │   ├── github-label-write.schema.json
│   │   ├── github-managed-issue-write.schema.json
│   │   ├── projection-approval.schema.json
│   │   ├── projection-operation.schema.json
│   │   ├── projection-preview.schema.json
│   │   ├── projection-reconciliation.schema.json
│   │   └── projection-result.schema.json
│   ├── store
│   │   ├── admitted-object.schema.json
│   │   ├── candidate-object.schema.json
│   │   ├── migration-record.schema.json
│   │   ├── mutation-envelope.schema.json
│   │   ├── object-history.schema.json
│   │   ├── recalculation-trigger.schema.json
│   │   └── store-event.schema.json
│   └── worker
│       ├── gpu-advisory-request.schema.json
│       ├── gpu-advisory-response.schema.json
│       ├── local-advisory-result.schema.json
│       ├── local-advisory-submission.schema.json
│       ├── provider-config.schema.json
│       ├── worker-authority.schema.json
│       ├── worker-result.schema.json
│       └── worker-submission.schema.json
├── scripts
│   ├── check-wire-casing.sh
│   ├── generate-typescript.mjs
│   ├── generate-typescript.sh
│   └── validate-all.sh
├── tools
│   ├── schema-index
│   │   ├── Cargo.toml
│   │   └── src
│   │       └── main.rs
│   └── validate-fixtures
│       ├── Cargo.toml
│       └── src
│           └── main.rs
└── tsconfig.json

101 directories, 252 files
```

### ubu-core: `tree -I target`

```text
.
├── CHANGELOG.md
├── CODEGEN.md
├── CONTRACT.md
├── CONTRIBUTING.md
├── Cargo.lock
├── Cargo.toml
├── LICENSE
├── README.md
├── SECURITY.md
├── build.rs
├── clippy.toml
├── docs
│   └── ADVISORY_CANDIDATE.md
├── fixtures
│   ├── README.md
│   └── placeholders
│       ├── invalid
│       │   ├── common
│       │   │   └── id
│       │   │       └── setting-wrong-prefix.json
│       │   ├── core
│       │   │   ├── advisory-candidate
│       │   │   │   ├── missing-required-field.json
│       │   │   │   ├── out-of-range-confidence.json
│       │   │   │   └── unknown-lifecycle-state.json
│       │   │   ├── device-registration
│       │   │   │   ├── missing-required-field.json
│       │   │   │   ├── non-identity-prefix.json
│       │   │   │   └── zone-id-array.json
│       │   │   ├── objective
│       │   │   │   ├── planned-without-range.json
│       │   │   │   ├── recurrence-on-one-time.json
│       │   │   │   ├── routine-with-priority.json
│       │   │   │   ├── template-without-recurrence.json
│       │   │   │   ├── unknown-rule-kind.json
│       │   │   │   └── weekly-without-weekdays.json
│       │   │   ├── preference
│       │   │   │   ├── incomplete-pair.json
│       │   │   │   ├── mixed-pair.json
│       │   │   │   └── unknown-order.json
│       │   │   └── task
│       │   │       ├── allowed-time-range-missing-latest-finish.json
│       │   │       ├── duplicate-tags.json
│       │   │       ├── invalid-duration-order.json
│       │   │       ├── static-window-missing-end.json
│       │   │       └── static-with-allowed-time-range.json
│       │   ├── store
│       │   │   └── mutation-envelope
│       │   │       ├── leading-zero-version.json
│       │   │       └── malformed-version-reference.json
│       │   └── worker
│       │       └── local-advisory-result
│       │           └── timeout-with-candidates.json
│       └── valid
│           ├── common
│           │   └── id
│           │       └── setting-id.json
│           ├── core
│           │   ├── advisory-candidate
│           │   │   ├── deferred.json
│           │   │   ├── proposed-tag.json
│           │   │   ├── redacted-payload.json
│           │   │   └── resurfaced.json
│           │   ├── device-registration
│           │   │   ├── phase1b-single-device.json
│           │   │   └── revoked.json
│           │   ├── external-reference
│           │   │   └── basic.json
│           │   ├── log-entry
│           │   │   └── basic.json
│           │   ├── objective
│           │   │   ├── basic.json
│           │   │   ├── routine-planned.json
│           │   │   └── routine-static.json
│           │   ├── preference
│           │   │   ├── objective-pair.json
│           │   │   └── task-pair.json
│           │   ├── setting
│           │   │   └── basic.json
│           │   ├── suppression-record
│           │   │   └── rejected-tag.json
│           │   └── task
│           │       ├── basic.json
│           │       ├── non-capacity.json
│           │       ├── routine-occurrence.json
│           │       ├── static-task.json
│           │       ├── tagged.json
│           │       ├── with-allowed-time-range.json
│           │       ├── with-category-tag.json
│           │       └── with-drift-fields.json
│           ├── planning
│           │   ├── planning-request
│           │   │   └── basic.json
│           │   ├── planning-response
│           │   │   └── basic.json
│           │   ├── repair-request
│           │   │   └── basic.json
│           │   └── repair-response
│           │       └── basic.json
│           ├── projection
│           │   └── projection-preview
│           │       └── basic.json
│           ├── store
│           │   └── mutation-envelope
│           │       ├── basic.json
│           │       ├── create-absence-precondition.json
│           │       ├── overnight-advisory.json
│           │       └── with-policy-versions.json
│           └── worker
│               ├── gpu-advisory-request
│               │   └── basic.json
│               ├── gpu-advisory-response
│               │   └── basic.json
│               ├── local-advisory-result
│               │   ├── ok.json
│               │   ├── partial.json
│               │   └── timeout.json
│               └── local-advisory-submission
│                   └── tag-proposal.json
├── rust-toolchain.toml
├── schemas-ref
│   ├── CHANGELOG.md
│   ├── CODEGEN.md
│   ├── CONTRACT.md
│   ├── CONTRIBUTING.md
│   ├── Cargo.lock
│   ├── Cargo.toml
│   ├── LICENSE
│   ├── PLANNING_KERNEL_CONTRACT.md
│   ├── README.md
│   ├── SECURITY.md
│   ├── clippy.toml
│   ├── fixtures
│   │   ├── invalid
│   │   │   ├── api
│   │   │   │   ├── human-complete-plan-quality
│   │   │   │   │   └── character-judgment.json
│   │   │   │   └── risk-report
│   │   │   │       └── legacy-items.json
│   │   │   ├── artifact
│   │   │   ├── common
│   │   │   │   ├── authority-source
│   │   │   │   │   ├── object-user-value.json
│   │   │   │   │   └── unknown-value.json
│   │   │   │   ├── id
│   │   │   │   │   ├── container-wrong-prefix.json
│   │   │   │   │   ├── external-event-wrong-prefix.json
│   │   │   │   │   ├── hyphenated-suffix.json
│   │   │   │   │   ├── identity-wrong-prefix.json
│   │   │   │   │   ├── preference-wrong-prefix.json
│   │   │   │   │   ├── relationship-wrong-prefix.json
│   │   │   │   │   ├── setting-wrong-prefix.json
│   │   │   │   │   ├── universe-state-wrong-prefix.json
│   │   │   │   │   ├── uppercase-suffix.json
│   │   │   │   │   ├── wrong-length.json
│   │   │   │   │   ├── wrong-prefix.json
│   │   │   │   │   ├── wrong-variant-bit.json
│   │   │   │   │   └── wrong-version-bit.json
│   │   │   │   ├── policy-summary
│   │   │   │   │   └── unknown-member.json
│   │   │   │   ├── provenance
│   │   │   │   │   └── github-event-authority.json
│   │   │   │   └── timestamp
│   │   │   │       └── naive.json
│   │   │   ├── core
│   │   │   │   ├── advisory-candidate
│   │   │   │   │   ├── missing-required-field.json
│   │   │   │   │   ├── out-of-range-confidence.json
│   │   │   │   │   └── unknown-lifecycle-state.json
│   │   │   │   ├── device-registration
│   │   │   │   │   ├── missing-required-field.json
│   │   │   │   │   ├── non-identity-prefix.json
│   │   │   │   │   └── zone-id-array.json
│   │   │   │   ├── external-reference
│   │   │   │   │   └── bad-url.json
│   │   │   │   ├── log-entry
│   │   │   │   │   ├── compartment-boundary-missing-authority-source.json
│   │   │   │   │   ├── compartment-boundary-missing-provenance.json
│   │   │   │   │   └── empty-message.json
│   │   │   │   ├── objective
│   │   │   │   │   ├── missing-status.json
│   │   │   │   │   ├── planned-without-range.json
│   │   │   │   │   ├── recurrence-on-one-time.json
│   │   │   │   │   ├── routine-with-priority.json
│   │   │   │   │   ├── template-without-recurrence.json
│   │   │   │   │   ├── unknown-rule-kind.json
│   │   │   │   │   └── weekly-without-weekdays.json
│   │   │   │   ├── precondition
│   │   │   │   │   └── unknown-predicate.json
│   │   │   │   ├── preference
│   │   │   │   │   ├── incomplete-pair.json
│   │   │   │   │   ├── mixed-pair.json
│   │   │   │   │   └── unknown-order.json
│   │   │   │   ├── snapshot
│   │   │   │   │   └── stale-tolerance-fields.json
│   │   │   │   ├── task
│   │   │   │   │   ├── active-with-moot-reason.json
│   │   │   │   │   ├── allowed-time-range-missing-latest-finish.json
│   │   │   │   │   ├── duplicate-correlation-group.json
│   │   │   │   │   ├── duplicate-tags.json
│   │   │   │   │   ├── effects-unknown-field.json
│   │   │   │   │   ├── invalid-duration-order.json
│   │   │   │   │   ├── invalid-duration-ordering.json
│   │   │   │   │   ├── moot-without-reason.json
│   │   │   │   │   ├── static-window-missing-end.json
│   │   │   │   │   ├── static-with-allowed-time-range.json
│   │   │   │   │   └── unknown-field.json
│   │   │   │   └── universe-state-mutation
│   │   │   │       └── missing-payload.json
│   │   │   ├── github
│   │   │   │   └── github-issue-source
│   │   │   │       └── bad-number.json
│   │   │   ├── projection
│   │   │   │   ├── projection-approval
│   │   │   │   │   └── bad-approved.json
│   │   │   │   ├── projection-preview
│   │   │   │   │   └── empty-operations-item.json
│   │   │   │   └── projection-result
│   │   │   │       └── bad-status.json
│   │   │   ├── store
│   │   │   │   ├── admitted-object
│   │   │   │   │   └── missing-provenance-authority-source.json
│   │   │   │   ├── mutation-envelope
│   │   │   │   │   ├── authoritative-execution-context.json
│   │   │   │   │   ├── leading-zero-version.json
│   │   │   │   │   ├── malformed-version-reference.json
│   │   │   │   │   └── missing-required-field.json
│   │   │   │   └── recalculation-trigger
│   │   │   │       ├── missing-trigger-type.json
│   │   │   │       └── old-reason.json
│   │   │   └── worker
│   │   │       ├── gpu-advisory-request
│   │   │       │   └── empty-workload.json
│   │   │       ├── gpu-advisory-response
│   │   │       │   └── bad-recommendation.json
│   │   │       ├── local-advisory-result
│   │   │       │   ├── timeout-with-candidates.json
│   │   │       │   └── unknown-status.json
│   │   │       └── local-advisory-submission
│   │   │           └── empty-capability-set.json
│   │   └── valid
│   │       ├── api
│   │       │   ├── human-complete-plan-quality
│   │       │   │   └── all-signals.json
│   │       │   └── risk-report
│   │       │       └── categorized-findings.json
│   │       ├── artifact
│   │       ├── common
│   │       │   ├── authority-source
│   │       │   │   └── user.json
│   │       │   ├── id
│   │       │   │   ├── container-id.json
│   │       │   │   ├── external-event-id.json
│   │       │   │   ├── identity-id.json
│   │       │   │   ├── preference-id.json
│   │       │   │   ├── relationship-id.json
│   │       │   │   ├── setting-id.json
│   │       │   │   ├── task-id.json
│   │       │   │   └── universe-state-id.json
│   │       │   ├── policy-summary
│   │       │   │   └── guardrail-members.json
│   │       │   ├── provenance
│   │       │   │   ├── system-github-source-ref.json
│   │       │   │   └── user.json
│   │       │   └── timestamp
│   │       │       └── offset.json
│   │       ├── core
│   │       │   ├── advisory-candidate
│   │       │   │   ├── deferred.json
│   │       │   │   ├── proposed-tag.json
│   │       │   │   ├── redacted-payload.json
│   │       │   │   └── resurfaced.json
│   │       │   ├── device-registration
│   │       │   │   ├── phase1b-single-device.json
│   │       │   │   └── revoked.json
│   │       │   ├── external-reference
│   │       │   │   └── basic.json
│   │       │   ├── log-entry
│   │       │   │   ├── basic.json
│   │       │   │   └── compartment-boundary-decided.json
│   │       │   ├── objective
│   │       │   │   ├── basic.json
│   │       │   │   ├── routine-planned.json
│   │       │   │   └── routine-static.json
│   │       │   ├── precondition
│   │       │   │   └── tree.json
│   │       │   ├── preference
│   │       │   │   ├── objective-pair.json
│   │       │   │   └── task-pair.json
│   │       │   ├── setting
│   │       │   │   └── basic.json
│   │       │   ├── snapshot
│   │       │   │   ├── bootstrap-defaults.json
│   │       │   │   └── live-observation.json
│   │       │   ├── suppression-record
│   │       │   │   └── rejected-tag.json
│   │       │   ├── task
│   │       │   │   ├── basic.json
│   │       │   │   ├── default-duration.json
│   │       │   │   ├── fixed-duration.json
│   │       │   │   ├── moot.json
│   │       │   │   ├── non-capacity.json
│   │       │   │   ├── routine-occurrence.json
│   │       │   │   ├── shifted-lognormal-duration.json
│   │       │   │   ├── static-task.json
│   │       │   │   ├── tagged.json
│   │       │   │   ├── with-allowed-time-range.json
│   │       │   │   ├── with-category-tag.json
│   │       │   │   ├── with-drift-fields.json
│   │       │   │   ├── with-effects.json
│   │       │   │   └── with-preconditions.json
│   │       │   ├── universe-state
│   │       │   │   └── populated.json
│   │       │   └── universe-state-mutation
│   │       │       ├── add-membership.json
│   │       │       ├── append-event-marker.json
│   │       │       ├── clear-fact.json
│   │       │       ├── decrement-numeric.json
│   │       │       ├── increment-numeric.json
│   │       │       ├── remove-membership.json
│   │       │       └── set-fact.json
│   │       ├── github
│   │       │   └── github-issue-source
│   │       │       └── basic.json
│   │       ├── planning
│   │       │   ├── affect-profile
│   │       │   │   └── bootstrap-default-review-priors.json
│   │       │   ├── plan
│   │       │   │   └── superseding-timed.json
│   │       │   └── plan-step
│   │       │       └── timed.json
│   │       ├── projection
│   │       │   ├── projection-approval
│   │       │   │   └── basic.json
│   │       │   ├── projection-preview
│   │       │   │   └── basic.json
│   │       │   └── projection-result
│   │       │       └── basic.json
│   │       ├── store
│   │       │   ├── admitted-object
│   │       │   │   └── basic.json
│   │       │   ├── mutation-envelope
│   │       │   │   ├── basic.json
│   │       │   │   ├── create-absence-precondition.json
│   │       │   │   ├── overnight-advisory.json
│   │       │   │   └── with-policy-versions.json
│   │       │   └── recalculation-trigger
│   │       │       └── basic.json
│   │       └── worker
│   │           ├── gpu-advisory-request
│   │           │   └── basic.json
│   │           ├── gpu-advisory-response
│   │           │   └── basic.json
│   │           ├── local-advisory-result
│   │           │   ├── ok.json
│   │           │   ├── partial.json
│   │           │   └── timeout.json
│   │           └── local-advisory-submission
│   │               └── tag-proposal.json
│   ├── generated
│   │   └── typescript
│   │       └── README.md
│   ├── package-lock.json
│   ├── package.json
│   ├── rust-toolchain.toml
│   ├── schemas
│   │   ├── api
│   │   │   ├── bootstrap-request.schema.json
│   │   │   ├── bootstrap-response.schema.json
│   │   │   ├── human-complete-plan-quality.schema.json
│   │   │   ├── human-complete-report.schema.json
│   │   │   ├── next-action-response.schema.json
│   │   │   ├── risk-report.schema.json
│   │   │   └── user-action-request.schema.json
│   │   ├── artifact
│   │   │   ├── artifact-manifest.schema.json
│   │   │   ├── claim-register.schema.json
│   │   │   ├── evidence-index.schema.json
│   │   │   ├── export-review.schema.json
│   │   │   └── publication-plan.schema.json
│   │   ├── common
│   │   │   ├── authority-source.schema.json
│   │   │   ├── compartment-label.schema.json
│   │   │   ├── duration.schema.json
│   │   │   ├── error.schema.json
│   │   │   ├── id-registry.schema.json
│   │   │   ├── id.schema.json
│   │   │   ├── money.schema.json
│   │   │   ├── object-ref.schema.json
│   │   │   ├── policy-summary.schema.json
│   │   │   ├── provenance.schema.json
│   │   │   ├── source-ref.schema.json
│   │   │   └── timestamp.schema.json
│   │   ├── core
│   │   │   ├── advisory-candidate.schema.json
│   │   │   ├── automation-worker.schema.json
│   │   │   ├── compartment.schema.json
│   │   │   ├── container.schema.json
│   │   │   ├── device-registration.schema.json
│   │   │   ├── external-event.schema.json
│   │   │   ├── external-reference.schema.json
│   │   │   ├── identity.schema.json
│   │   │   ├── log-entry.schema.json
│   │   │   ├── moot-reason-code.schema.json
│   │   │   ├── objective.schema.json
│   │   │   ├── precondition.schema.json
│   │   │   ├── preference.schema.json
│   │   │   ├── relationship.schema.json
│   │   │   ├── setting.schema.json
│   │   │   ├── snapshot.schema.json
│   │   │   ├── suppression-record.schema.json
│   │   │   ├── task-status.schema.json
│   │   │   ├── task.schema.json
│   │   │   ├── universe-state-mutation.schema.json
│   │   │   ├── universe-state.schema.json
│   │   │   └── work-item.schema.json
│   │   ├── github
│   │   │   ├── github-ci-event-source.schema.json
│   │   │   ├── github-comment-source.schema.json
│   │   │   ├── github-import-batch.schema.json
│   │   │   ├── github-issue-source.schema.json
│   │   │   ├── github-label-source.schema.json
│   │   │   ├── github-milestone-source.schema.json
│   │   │   ├── github-pr-source.schema.json
│   │   │   ├── github-repository-source.schema.json
│   │   │   └── github-review-source.schema.json
│   │   ├── planning
│   │   │   ├── affect-profile.schema.json
│   │   │   ├── calendar.schema.json
│   │   │   ├── explanation-fragment.schema.json
│   │   │   ├── plan-step.schema.json
│   │   │   └── plan.schema.json
│   │   ├── projection
│   │   │   ├── github-comment-write.schema.json
│   │   │   ├── github-label-write.schema.json
│   │   │   ├── github-managed-issue-write.schema.json
│   │   │   ├── projection-approval.schema.json
│   │   │   ├── projection-operation.schema.json
│   │   │   ├── projection-preview.schema.json
│   │   │   ├── projection-reconciliation.schema.json
│   │   │   └── projection-result.schema.json
│   │   ├── store
│   │   │   ├── admitted-object.schema.json
│   │   │   ├── candidate-object.schema.json
│   │   │   ├── migration-record.schema.json
│   │   │   ├── mutation-envelope.schema.json
│   │   │   ├── object-history.schema.json
│   │   │   ├── recalculation-trigger.schema.json
│   │   │   └── store-event.schema.json
│   │   └── worker
│   │       ├── gpu-advisory-request.schema.json
│   │       ├── gpu-advisory-response.schema.json
│   │       ├── local-advisory-result.schema.json
│   │       ├── local-advisory-submission.schema.json
│   │       ├── provider-config.schema.json
│   │       ├── worker-authority.schema.json
│   │       ├── worker-result.schema.json
│   │       └── worker-submission.schema.json
│   ├── scripts
│   │   ├── check-wire-casing.sh
│   │   ├── generate-typescript.mjs
│   │   ├── generate-typescript.sh
│   │   └── validate-all.sh
│   ├── tools
│   │   ├── schema-index
│   │   │   ├── Cargo.toml
│   │   │   └── src
│   │   │       └── main.rs
│   │   └── validate-fixtures
│   │       ├── Cargo.toml
│   │       └── src
│   │           └── main.rs
│   └── tsconfig.json
├── src
│   ├── advisory_candidate.rs
│   ├── authority.rs
│   ├── compartment_label.rs
│   ├── core
│   │   ├── automation_worker.rs
│   │   ├── compartment.rs
│   │   ├── container.rs
│   │   ├── external_event.rs
│   │   ├── external_reference.rs
│   │   ├── identity.rs
│   │   ├── log_entry.rs
│   │   ├── mod.rs
│   │   ├── objective.rs
│   │   ├── preference.rs
│   │   ├── relationship.rs
│   │   ├── routine.rs
│   │   ├── setting.rs
│   │   ├── snapshot.rs
│   │   ├── task.rs
│   │   ├── universe_state.rs
│   │   └── work_item.rs
│   ├── device.rs
│   ├── errors.rs
│   ├── github
│   │   ├── ci_event_source.rs
│   │   ├── issue_source.rs
│   │   ├── mod.rs
│   │   ├── pr_source.rs
│   │   └── repository_source.rs
│   ├── id_registry.rs
│   ├── ids.rs
│   ├── lib.rs
│   ├── object_ref.rs
│   ├── planning
│   │   ├── calendar.rs
│   │   ├── diagnostics.rs
│   │   ├── explanation.rs
│   │   ├── mod.rs
│   │   ├── plan.rs
│   │   ├── plan_step.rs
│   │   ├── planning_request.rs
│   │   ├── planning_response.rs
│   │   ├── repair_request.rs
│   │   └── repair_response.rs
│   ├── policy_summary.rs
│   ├── projection
│   │   ├── approval.rs
│   │   ├── legitimizer.rs
│   │   ├── mod.rs
│   │   ├── operation.rs
│   │   ├── preview.rs
│   │   └── result.rs
│   ├── provenance.rs
│   ├── serde_helpers.rs
│   ├── source_ref.rs
│   ├── store
│   │   ├── admitted_object.rs
│   │   ├── candidate_object.rs
│   │   ├── mod.rs
│   │   ├── mutation_envelope.rs
│   │   └── recalculation_trigger.rs
│   ├── time.rs
│   ├── validation.rs
│   └── worker
│       ├── authority.rs
│       ├── gpu_advisory.rs
│       ├── local_advisory.rs
│       ├── mod.rs
│       └── submission.rs
└── tests
    ├── authority_source.rs
    ├── id_validation.rs
    ├── ranges_preferences.rs
    ├── routine.rs
    ├── schema_fixture_roundtrip.rs
    ├── serialization.rs
    ├── task_lifecycle.rs
    ├── task_mapping_fields.rs
    └── timestamp_validation.rs

153 directories, 401 files
```

### ubu-store: `tree -I target`

```text
.
├── CHANGELOG.md
├── CODEGEN.md
├── CONTRACT.md
├── CONTRIBUTING.md
├── Cargo.lock
├── Cargo.toml
├── LICENSE
├── README.md
├── SECURITY.md
├── clippy.toml
├── docs
│   ├── CANONICAL_WRITER_AUDIT.md
│   └── PHASE1_STORE_CONTRACT.md
├── examples
│   ├── admit_task.rs
│   └── init_store.rs
├── fixtures
│   ├── invalid
│   └── valid
├── migrations
│   ├── 0001_initial.sql
│   ├── 0002_mutation_envelopes.sql
│   ├── 0003_advisory_candidates.sql
│   └── 0004_task_occurrence_key.sql
├── rust-toolchain.toml
├── src
│   ├── admission.rs
│   ├── api
│   │   ├── admission.rs
│   │   ├── mod.rs
│   │   ├── query.rs
│   │   ├── review.rs
│   │   └── store.rs
│   ├── candidates.rs
│   ├── compartment_gate.rs
│   ├── db.rs
│   ├── errors.rs
│   ├── lib.rs
│   ├── migrations.rs
│   ├── models
│   │   ├── calendar_record.rs
│   │   ├── candidate_record.rs
│   │   ├── external_reference_record.rs
│   │   ├── log_record.rs
│   │   ├── mod.rs
│   │   ├── object_record.rs
│   │   ├── plan_record.rs
│   │   ├── projection_record.rs
│   │   ├── recorded_mutation.rs
│   │   └── worker_submission_record.rs
│   ├── provenance_gate.rs
│   ├── queries.rs
│   ├── recalculation.rs
│   ├── replay.rs
│   └── transactions.rs
└── tests
    ├── admission_invariants.rs
    ├── admit_object.rs
    ├── advisory_candidates.rs
    ├── append_log.rs
    ├── common
    │   └── mod.rs
    ├── compartment_gate.rs
    ├── external_reference.rs
    ├── init_store.rs
    ├── mutation_envelope_admission.rs
    ├── plan_calendar.rs
    ├── preferences.rs
    ├── recalculation_trigger.rs
    ├── reject_invalid_object.rs
    ├── reject_prefix_type_mismatch.rs
    ├── replay_state.rs
    ├── routines.rs
    ├── task_duration_estimate.rs
    ├── task_mapping_fields.rs
    └── universe_state.rs

12 directories, 65 files
```

### ubu-github-adapter: `tree -I target`

```text
.
├── CHANGELOG.md
├── CODEGEN.md
├── CONTRACT.md
├── CONTRIBUTING.md
├── Cargo.lock
├── Cargo.toml
├── LICENSE
├── README.md
├── SECURITY.md
├── clippy.toml
├── examples
│   ├── import_fixture.rs
│   └── preview_projection.rs
├── fixtures
│   ├── github
│   │   ├── ci-small.json
│   │   ├── issues-small.json
│   │   ├── prs-small.json
│   │   ├── repo-fresh.json
│   │   └── repo-small.json
│   └── projection
│       ├── comment-preview.json
│       ├── label-preview.json
│       └── managed-issue-preview.json
├── rust-toolchain.toml
├── src
│   ├── approval.rs
│   ├── auth.rs
│   ├── candidate_mapping.rs
│   ├── cli
│   │   ├── apply_projection.rs
│   │   ├── import_fixture.rs
│   │   ├── import_live.rs
│   │   ├── mod.rs
│   │   └── preview_projection.rs
│   ├── client.rs
│   ├── errors.rs
│   ├── fixture.rs
│   ├── lib.rs
│   ├── markers.rs
│   ├── normalize.rs
│   ├── projection
│   │   ├── comments.rs
│   │   ├── label_write.rs
│   │   ├── labels.rs
│   │   ├── managed_issues.rs
│   │   ├── mod.rs
│   │   ├── operations.rs
│   │   ├── preview.rs
│   │   ├── reconciliation.rs
│   │   └── result.rs
│   ├── projection.rs
│   ├── reconcile.rs
│   ├── sources
│   │   ├── ci_event.rs
│   │   ├── comment.rs
│   │   ├── issue.rs
│   │   ├── label.rs
│   │   ├── milestone.rs
│   │   ├── mod.rs
│   │   ├── pull_request.rs
│   │   ├── repository.rs
│   │   └── review.rs
│   └── write.rs
└── tests
    ├── candidate_mapping.rs
    ├── fixture_validation.rs
    ├── import_ci_fixture.rs
    ├── import_issue_fixture.rs
    ├── import_live.rs
    ├── import_pr_fixture.rs
    ├── managed_issue_marker.rs
    ├── managed_label_write.rs
    ├── projection_approval.rs
    ├── projection_preview.rs
    └── reconciliation.rs

10 directories, 67 files
```

### ubu-planning-kernel: `tree -I target`

```text
.
├── CHANGELOG.md
├── CODEGEN.md
├── CONTRACT.md
├── CONTRIBUTING.md
├── Cargo.lock
├── Cargo.toml
├── LICENSE
├── README.md
├── SECURITY.md
├── clippy.toml
├── crates
│   ├── ubu-planning-advisory-protocol
│   │   ├── Cargo.toml
│   │   └── src
│   │       ├── lib.rs
│   │       ├── noop.rs
│   │       └── process.rs
│   ├── ubu-planning-cli
│   │   ├── Cargo.toml
│   │   └── src
│   │       ├── commands
│   │       │   ├── advisory.rs
│   │       │   ├── mod.rs
│   │       │   ├── plan.rs
│   │       │   ├── repair.rs
│   │       │   └── validate.rs
│   │       └── main.rs
│   ├── ubu-planning-core
│   │   ├── Cargo.toml
│   │   ├── src
│   │   │   ├── diagnostics.rs
│   │   │   ├── explanations.rs
│   │   │   ├── graph.rs
│   │   │   ├── legitimization.rs
│   │   │   ├── lib.rs
│   │   │   ├── request.rs
│   │   │   ├── response.rs
│   │   │   ├── rollout.rs
│   │   │   ├── scoring.rs
│   │   │   ├── strategy.rs
│   │   │   └── validation.rs
│   │   └── tests
│   │       ├── advisory_noop.rs
│   │       ├── affect_golden_fixtures.rs
│   │       ├── chunked_sweep.rs
│   │       ├── deterministic_seed.rs
│   │       ├── end_to_end_plan.rs
│   │       ├── golden_fixtures.rs
│   │       ├── legitimization.rs
│   │       ├── phase_a.rs
│   │       ├── phase_c1.rs
│   │       ├── rollout_golden_fixtures.rs
│   │       ├── scoring_selection_golden_fixtures.rs
│   │       ├── skeleton_failure.rs
│   │       ├── static_placement.rs
│   │       └── status_vocabulary.rs
│   └── ubu-planning-cpu
│       ├── Cargo.toml
│       └── src
│           ├── candidate_generation.rs
│           ├── chunked.rs
│           ├── default_selection.rs
│           ├── lib.rs
│           ├── repair.rs
│           └── skeleton.rs
├── fixtures
│   ├── advisory
│   │   ├── noop-request.json
│   │   └── noop-response.json
│   └── planning
│       ├── golden
│       │   ├── README.md
│       │   ├── affect-legitimization.json
│       │   ├── rollout-c2.json
│       │   ├── rollout-degraded-c2.json
│       │   ├── scoring-selection-c1.json
│       │   └── skeleton-phase-a.json
│       ├── invalid
│       │   ├── cyclic-dependency.json
│       │   ├── impossible-window.json
│       │   ├── missing-dependency.json
│       │   ├── missing-schema-version.json
│       │   ├── stale-affect.json
│       │   └── unknown-schema-version.json
│       └── valid
│           ├── affect-break-required.json
│           ├── dependency-chain.json
│           ├── simple-success.json
│           └── static-anchor.json
├── gpu-advisory
│   ├── README.md
│   ├── pyproject.toml
│   ├── src
│   │   └── ubu_gpu_advisory
│   │       ├── __init__.py
│   │       ├── __pycache__
│   │       │   ├── __init__.cpython-313.pyc
│   │       │   ├── noop.cpython-313.pyc
│   │       │   └── protocol.cpython-313.pyc
│   │       ├── main.py
│   │       ├── noop.py
│   │       └── protocol.py
│   └── tests
│       └── test_noop_protocol.py
├── rust-toolchain.toml
└── tests
    ├── advisory_noop.rs
    ├── deterministic_seed.rs
    ├── end_to_end_plan.rs
    ├── legitimization.rs
    ├── skeleton_failure.rs
    └── status_vocabulary.rs

24 directories, 89 files
```

### ubu-orchestrator: `tree -I target`

```text
.
├── CHANGELOG.md
├── CODEGEN.md
├── CONTRACT.md
├── CONTRIBUTING.md
├── Cargo.lock
├── Cargo.toml
├── LICENSE
├── README.md
├── SECURITY.md
├── clippy.toml
├── docs
│   ├── CATEGORY_COLOUR.md
│   ├── ENVELOPE_CALL_SITE_AUDIT.md
│   ├── P1B-10b_VERIFICATION.md
│   ├── P1B-11_VERIFICATION.md
│   ├── P1B-12_VERIFICATION.md
│   ├── P1B-13_VERIFICATION.md
│   ├── P1B-14_VERIFICATION.md
│   ├── P1B-15_VERIFICATION.md
│   ├── P1B-16_VERIFICATION.md
│   ├── P1B-17_VERIFICATION.md
│   ├── P1B-18_VERIFICATION.md
│   ├── PLANNING_PRIORITY.md
│   ├── PLANNING_TIME.md
│   └── QUICK_UBU_IMPORT.md
├── examples
│   ├── generate_openapi.rs
│   └── run_fixture_loop.rs
├── fixtures
│   ├── fixture-loop
│   │   ├── expected-next-action.json
│   │   └── github-small.json
│   ├── github
│   │   └── issues-small.json
│   └── quick-ubu
│       └── snapshot-small.json
├── openapi
│   ├── README.md
│   └── openapi.generated.json
├── rust-toolchain.toml
├── src
│   ├── adapters
│   │   ├── mod.rs
│   │   └── planner_adapter.rs
│   ├── api
│   │   ├── advisory.rs
│   │   ├── bootstrap.rs
│   │   ├── calendar.rs
│   │   ├── desktop.rs
│   │   ├── github.rs
│   │   ├── health.rs
│   │   ├── mod.rs
│   │   ├── next_action.rs
│   │   ├── planning.rs
│   │   ├── projection.rs
│   │   ├── quick_ubu.rs
│   │   ├── recalculation.rs
│   │   ├── reports.rs
│   │   └── user_action.rs
│   ├── category_palette.rs
│   ├── config.rs
│   ├── device_registration.rs
│   ├── errors.rs
│   ├── instance_mode.rs
│   ├── lib.rs
│   ├── main.rs
│   ├── openapi.rs
│   ├── planning_time.rs
│   ├── reports
│   │   ├── human_complete_report.rs
│   │   ├── mod.rs
│   │   ├── planning_analysis.rs
│   │   └── risk_report.rs
│   ├── router.rs
│   ├── services
│   │   ├── advisory_service.rs
│   │   ├── bootstrap_service.rs
│   │   ├── desktop_session_service.rs
│   │   ├── import_service.rs
│   │   ├── log_service.rs
│   │   ├── mod.rs
│   │   ├── next_action_service.rs
│   │   ├── planning_service.rs
│   │   ├── projection_service.rs
│   │   ├── proposal_applier.rs
│   │   ├── quick_ubu_import.rs
│   │   ├── recalculation_service.rs
│   │   ├── report_service.rs
│   │   └── task_priority.rs
│   ├── state.rs
│   └── tracing.rs
└── tests
    ├── advisory_controller.rs
    ├── bootstrap.rs
    ├── device_registration.rs
    ├── fixture_loop.rs
    ├── github_import.rs
    ├── health.rs
    ├── mutation_envelopes.rs
    ├── next_action.rs
    ├── planning_o9.rs
    ├── planning_request_builder.rs
    ├── planning_time.rs
    ├── projection_approval.rs
    ├── projection_preview.rs
    ├── quick_ubu_import.rs
    ├── ranges_priority.rs
    ├── reports.rs
    ├── static_category.rs
    ├── support
    │   └── advisory_fixture.rs
    └── user_actions.rs

15 directories, 98 files
```
