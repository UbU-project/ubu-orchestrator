# P1B-12 advisory review actions

## Implementation and dependency order

All nine sibling repositories started clean. Devshell was already at DS-1
`0499179b860dca24de24c7e37929f4b4d8ff7f17`; update-all fetched and reported all
existing repositories up to date, and gen-patch-config generated the local
package patches. Development and initial full tests used those patches offline.

Store A replaces caller-built suppression records with RejectionInput. The store
derives missing keys from candidate identity, builds the record using the core
builder and envelope provenance, and rejects conflicting supplied keys without
writes. Store B updates the writer audit and regression tests. The pushed B rev is
`7be1146e98f1bda39d3a2bb2bac04ade4d97b581` on `p1b-12-store-review-actions`.

Orchestrator C pins that exact revision in Cargo.toml and the committed-source
Cargo.lock. D adds the pure add_tag applier. E adds admit/reject/defer/resurface
services using fresh User envelopes. F registers HTTP routes and OpenAPI schemas.
G tests the entire stub-ingestion/review path and the admission concurrency
boundary. H records patched/unpatched verification. Each letter has its own commit.

## Behavior and regression coverage

- Admission reads normalized_proposal, accepts redacted payloads, requires a Tag
  operation and exactly one matching Task target, validates the Task, and writes
  the object/candidate/decision atomically through the existing store writer.
- An existing tag is not duplicated or rejected. The candidate becomes Admitted
  and no suppression record is created. Double submission fails on the reviewed
  candidate version without additional writes.
- Rejection preserves canonical state and creates readable suppression metadata.
  Defer requires resurfacing before admission, and resurfacing carries its trigger.
- Unsupported kinds/operations, malformed proposals and missing targets leave
  state untouched. HTTP diagnostics distinguish UnsupportedProposal,
  TargetNotFound, stale versions and invalid lifecycle transitions.
- The stale-target regression ingests through run_advisory, uses the production
  prepare_admission path, commits a competing Task update, then invokes the
  production prepared commit. It asserts a typed store PreconditionFailed, the
  unchanged concurrent Task, a Proposed candidate, no decision and no ledger write.
- All four pre-existing advisory controller test bodies are preserved (verified
  ignoring formatting); their stub/submission helpers moved to shared test support
  so the concurrency test uses the same ingestion path.
- Store regressions cover key derivation from Proposed with no key, canonical
  proposal member ordering, equivalent identity across different candidates,
  target scoping, key precedence/conflict, equality to the core builder and replay.

## Derived suppression key example

The rejection endpoint regression asserts this exact key:

```json
{"candidate_kind":"tag","normalized_proposal":{"operation":"add_tag","tag":"focus"},"target_refs":[{"id":"task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f","object_type":"Task"}]}
```

## Literal readings and implementation choices

- `src/services/advisory_service.rs:116` and `:265`: the request's
  observed_version is the candidate version. Section E requires reading the
  Task's current version at admission. Thus the stale-target case means a Task
  mutation between that service read and the atomic write, not an earlier change
  before the service is called. There is no target-version request field. The
  deterministic regression exercises that exact boundary without timing sleeps
  or a public test hook.
- `src/services/proposal_applier.rs:45`: non-empty means not the empty string,
  matching core Task validation; tags are not trimmed or case-folded.
- `src/services/proposal_applier.rs:59`: Task validation uses the existing core
  validate_task_lifecycle function, which includes validate_fields. Only tags are
  replaced in the cloned payload, preserving unrelated fields and representation.
- `src/services/proposal_applier.rs:56` and
  `src/services/advisory_service.rs:139`: already-present tags still use ordinary
  admission, so the store advances the Task version and the service sets the row's
  updated_at from the envelope. Task tags/payload remain unchanged in this case;
  no duplicate or suppression record is written.
- `src/services/advisory_service.rs:179`: rejection uses an empty observation map,
  empty evidence fingerprints and no caller key, allowing store key precedence.
  Retention policy is recorded; this ticket does not execute payload purging.
- `src/api/advisory.rs:83` and `:91`: request fields deserialize directly into
  core RetentionPolicy/ResurfaceTrigger. Local schema builders document their
  string enums because core deliberately has no OpenAPI dependency.
- `src/api/advisory.rs:127`: review precondition/lifecycle failures become HTTP
  409; missing targets/candidates are 404; unsupported proposals are 422; malformed
  proposal fields and rejection reasons are 400. Services retain typed store errors.
- Store `src/candidates.rs:268`: key input is a JSON object with candidate_kind,
  normalized_proposal and target_refs, using canonical_payload_bytes. Object keys
  are sorted recursively and target array order is preserved; no hashing or new
  dependency was added.
- Store `src/candidates.rs:261` and `:287`: conflicting supplied keys fail with
  SuppressionKeyConflict. Existing duplicate-key insertion semantics are retained:
  a second suppression with the same key fails rather than overwriting correction
  metadata. Deterministic derivation is compared in isolated stores in
  `tests/advisory_candidates.rs:513`; suppression matching remains out of scope.
- Store `src/candidates.rs:251`: the replay fingerprint now contains the new input
  rather than a caller-built record. Same envelope/input still replays without
  writes; changed input still conflicts. The impossible caller-provenance-mismatch
  tests were replaced by conflict and internally-built provenance assertions.
- `tests/support/advisory_fixture.rs:1`: pre-existing controller setup is shared
  with the private service concurrency test; tests still ingest via run_advisory
  and an in-memory stub transport. No live service calls are used in tests.
- `Cargo.toml:22` and `Cargo.lock:2347`: the store pin/lock were prepared with the
  orchestrator patch temporarily moved aside. No path lock entry was committed
  and registry dependency versions were preserved.

No core, schemas, adapter, kernel or devshell source changes, new dependencies,
additional proposal kinds, suppression matching, or other out-of-scope behavior
were implemented.

## H: verification results

Both full suites passed offline, patched and unpatched: store 90 tests and
orchestrator 84 tests. Sorted named outcomes match exactly between the two modes.
Patched commands were `CARGO_NET_OFFLINE=true cargo test`; unpatched tests used
`CARGO_NET_OFFLINE=true cargo test --locked`. The explicit unpatched orchestrator
`CARGO_NET_OFFLINE=true cargo build --locked` also passed.

Patched `CARGO_NET_OFFLINE=true cargo tree --locked -i ubu_core`:

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

Unpatched `CARGO_NET_OFFLINE=true cargo tree --locked -i ubu_core`:

```text
ubu_core v0.1.0 (https://github.com/UbU-project/ubu-core?rev=cef80fe0ea68bf49413b6e75673dcd41274c9d75#cef80fe0)
├── ubu_github_adapter v0.1.0 (https://github.com/UbU-project/ubu-github-adapter?rev=4b31c8c3c933a0b1cc8674a7d69849bf39484467#4b31c8c3)
│   └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_core v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=951d88ded8c95ed7cf5bfe815200dea0bdffd962#951d88de)
│   ├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
│   └── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=951d88ded8c95ed7cf5bfe815200dea0bdffd962#951d88de)
│       └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=951d88ded8c95ed7cf5bfe815200dea0bdffd962#951d88de) (*)
└── ubu_store v0.1.0 (https://github.com/UbU-project/ubu-store?rev=7be1146e98f1bda39d3a2bb2bac04ade4d97b581#7be1146e)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
```

The committed lockfile contains exactly one ubu_core source at cef80fe and
one ubu_store source at the pushed B revision. No path lockfile was committed.

The moved configs were restored after unpatched verification, then all generated
configs were removed as H requires. Sibling lockfiles were restored to their
original bytes, except the orchestrator retains its committed store pin update.
All other sibling HEADs and tracked files match the initial baseline.

Verbatim final 20 lines of the unpatched store test run:

```text
running 10 tests
test rejects_universe_state_with_non_ustate_id_prefix ... ok
test persist_universe_state_requires_existing_current_version ... ok
test rejects_universe_state_without_provenance_authority_source ... ok
test admits_universe_state_with_envelope_and_provenance_authority_source ... ok
test persists_updated_universe_state_as_new_current_version ... ok
test universe_ledger_failure_rolls_back_updated_object ... ok
test universe_requires_version_target_and_checks_read_preconditions ... ok
test admits_universe_state_and_round_trips_all_collections ... ok
test universe_envelope_update_replay_and_stale_precondition ... ok
test universe_payload_or_authority_change_conflicts_on_replay ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

   Doc-tests ubu_store

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

Verbatim final 20 lines of the unpatched orchestrator test run:

```text
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/user_actions.rs (target/debug/deps/user_actions-99b936fc7004ba07)

running 6 tests
test record_action_requires_known_schema_version ... ok
test complete_records_decision_log_and_transitions_task ... ok
test complete_without_effects_leaves_universe_state_unchanged ... ok
test override_records_user_override_without_transition ... ok
test complete_with_effects_but_no_universe_state_surfaces_diagnostic ... ok
test complete_applies_task_effects_to_universe_state ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

   Doc-tests ubu_orchestrator

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```
