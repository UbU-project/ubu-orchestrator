# P1B-16 verification

Implemented allowed Task ranges and pairwise Task Preferences. Named bootstrap
and affect settings now use `Setting` / `setting_`; `Preference` / `pref_` is the
same-kind pairwise relation. No dependency was added.

## Revisions and workflow

All sibling trees were clean before work. `update-all.sh` reported every repository
already current. Local generated Cargo patches were used during development.
Every changed repository uses `p1b-16-ranges-priority`; dependencies were pushed in
order. The design checkout was already `52ff841`, so the required reference was
read using `git show d2bc730:DESIGN.md` and `git show d2bc730:DECISIONS.md` without
changing its checkout. Devshell, UI and design have no ticket changes.

| Repository | Pushed dependency revision |
|---|---|
| ubu-schemas | `eb2614a31a3e44991bd86c971cb87aa2c4760c35` |
| ubu-core | `358086dba4575984beccdaa7f5cc0d457a276fe9` |
| ubu-store | `685b68e19d57014f7be355993286562644a80a2d` |
| ubu-github-adapter | `5664000e823aee6f35439d4ef15f218285768bb4` |
| ubu-planning-kernel | `e8045e2cefb6ccb0f8e20ce0ba22f2d20bfd5990` |

## Tests

All six repository suites passed: schemas validated 72 valid and 64 invalid
fixtures; core 123 tests, store 98, adapter 23, kernel 39 and orchestrator 127.
Adapter tests use in-memory fakes. Kernel golden fixtures and property/invariant
assertions all passed unchanged, including byte-exact skeleton, affect,
scoring/selection and rollout fixtures. Kernel source and fixtures are unchanged;
only its core dependency pin and lock resolution changed. The optional Python
advisory test also passed using a stdlib runner (no pytest installation needed).

The existing orchestrator suite passed after the Setting rename, then again with
ranges and priority. Both `static_category.rs` dependency behavior tests remain
unchanged and passed. New coverage includes eight pure priority unit tests and
nine API tests using the fixed planning clock. Full logs and literal `tree -I target`
outputs for all six repositories are retained in sibling `P1B-16-results/`.

Verbatim suite tails:

### schemas

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.04s
     Running `target/debug/validate-fixtures`
validated 72 valid fixtures
validated 64 invalid fixtures
```

### core

```text
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### store

```text
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

   Doc-tests ubu_store

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### adapter

```text
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_github_adapter

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### kernel

```text
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_planning_cpu

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### orchestrator

```text
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

   Doc-tests ubu_orchestrator

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.90s
```

## Dependency resolution (O)

Patched `cargo tree -i ubu_core`: one path source.

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

With `.cargo/config.toml` moved aside, `cargo build --locked` passed with these
committed sources. The lockfile was updated to the already-pushed manifest pins
before that locked check, without changing any registry dependency version.

```text
ubu_core v0.1.0 (https://github.com/UbU-project/ubu-core?rev=358086dba4575984beccdaa7f5cc0d457a276fe9#358086db)
├── ubu_github_adapter v0.1.0 (https://github.com/UbU-project/ubu-github-adapter?rev=5664000e823aee6f35439d4ef15f218285768bb4#5664000e)
│   └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_core v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=e8045e2cefb6ccb0f8e20ce0ba22f2d20bfd5990#e8045e2c)
│   ├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
│   └── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=e8045e2cefb6ccb0f8e20ce0ba22f2d20bfd5990#e8045e2c)
│       └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=e8045e2cefb6ccb0f8e20ce0ba22f2d20bfd5990#e8045e2c) (*)
└── ubu_store v0.1.0 (https://github.com/UbU-project/ubu-store?rev=685b68e19d57014f7be355993286562644a80a2d#685b68e1)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
```

Both kernel packages resolve to the same ticket kernel revision, and every consumer
resolves the single ticket core revision. The config was restored, then all four
generated configs were removed as required. The committed orchestrator lockfile
records this unpatched resolution.

## Builder order and semantics

1. Resolve the horizon and existing repair scope; partition participation and
   preserve each Task's original dependencies.
2. Intersect each capacity-occupying Dynamic Task's allowed range with the horizon.
   Preserve whole Static placements under the existing rules.
3. Push Dynamic starts using `absent_static_windows`, then apply the now floor.
4. Exclude `dependency_outside_horizon` first (test the absent prerequisite's end),
   then `task_unplaceable` using the received kernel model's `placement_seconds()`.
5. Exclude Dynamic dependents to a `prerequisite_unplaceable` fixpoint. Kept Static
   Tasks break propagation; other exclusion reasons keep existing behavior.
6. Drop edges to unplanned Tasks only after that fixpoint.
7. Layer eligible pairwise Preferences, assign request values, then topologically
   order ready Tasks by bucket, allowed latest finish and ID. Dispatch priority is
   always 1.0; caller body values default to 1.0.

The measure deliberately uses the fixed duration or log-normal mode, stricter than
D0276's minimum-duration floor (ticket lines 178–180). Individually oversized Tasks
are excluded, but greedy packing failures remain documented in PLANNING_TIME.md
and PLANNING_PRIORITY.md. No chunked search or kernel semantic change was made.

## Cycle response

Verbatim `task_priorities` and `diagnostics` from the cycle API test:

```json
{
  "diagnostics": [
    {
      "code": "preference_cycle",
      "message": "Preference cycle among Tasks [task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e01, task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e02, task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e03]; please resolve this high-priority consistency error"
    }
  ],
  "task_priorities": [
    {
      "bucket": 0,
      "bucket_count": 1,
      "normalized_rank": 0.0,
      "task_id": "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e01",
      "value": 1.0
    },
    {
      "bucket": 0,
      "bucket_count": 1,
      "normalized_rank": 0.0,
      "task_id": "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e02",
      "value": 1.0
    },
    {
      "bucket": 0,
      "bucket_count": 1,
      "normalized_rank": 0.0,
      "task_id": "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e03",
      "value": 1.0
    }
  ]
}
```

The test still produces a Plan. `task_priorities` is sorted by ID and response-only;
an admission-backed test verifies it and normalized rank are absent from the
stored Plan. Caller-supplied requests omit the empty response explanation.

## Rename-only existing test changes

| File:line | Test or helper |
|---|---|
| ubu-store/tests/admission_invariants.rs:31 | admits_setting_with_canonical_envelope (renamed) |
| ubu-store/tests/admission_invariants.rs:60 | rejects_payload_without_canonical_id |
| ubu-store/tests/admission_invariants.rs:87 | rejects_payload_id_that_does_not_match_record_id |
| ubu-store/tests/admission_invariants.rs:116 | rejects_non_positive_object_version |
| ubu-store/tests/admission_invariants.rs:174 | rejects_setting_without_authority_source (renamed) |
| ubu-orchestrator/tests/bootstrap.rs:13 | seed_admits_bootstrap_state_and_imports_selected_repo_tasks |
| ubu-orchestrator/tests/bootstrap.rs:136 | seed_rejects_second_run_without_duplicating_bootstrap_objects |
| ubu-orchestrator/tests/planning_o9.rs:434 | store_backed_request_uses_affect_settings_and_fresh_snapshot (renamed) |
| ubu-orchestrator/tests/planning_o9.rs:504 | stale_affect_snapshot_is_not_presented_as_current |
| ubu-orchestrator/tests/planning_time.rs:181 | kernel_legitimization_detects_seconds_of_staleness |
| ubu-orchestrator/tests/planning_o9.rs:922 | admit_setting helper (renamed) |
| ubu-orchestrator/tests/planning_time.rs:581 | admit_setting helper (renamed) |

The registry test at ubu-store/tests/admission_invariants.rs:11 additionally covers
both Setting and Preference. No behavioral assertion was weakened. The answer
`attention_preference` and its UniverseState fact retain their names.

## Audit and remaining follow-ups

The source/test grep across the six changed repositories found no named-setting
use of `ObjectType::Preference` or `"Preference"`. Remaining references describe
pairwise Preferences or their ID/type registry. There are no downstream `Task`
struct literals; construction uses `Task::new`. All three builder `TaskSpecBody`
literals include value: placeholders and non-capacity Statics retain 1.0, while
eligible specs receive derived values. Grep output is in `P1B-16-results/`.

`setting.schema.json` retains the old schema's shape. Stored bootstrap Settings
also contain `provenance`, which that schema does not list (ticket line 112);
fixing that discrepancy is out of scope. Core Setting adds the required `id`.

UI follow-up is still required: `ubu-ui/src/api/client.ts:127` and
`ubu-ui/src/routes/Bootstrap.tsx:129` read `preference_ids`, and
`ubu-ui/src/types/generated/index.d.ts:406` still declares the old named-setting
Preference. Update those consumers and regenerate UI types separately.

## Deviations and literal readings

- P1B-16_prompt.md:12: the design checkout was newer; read the specified commit
  directly rather than moving the read-only checkout.
- P1B-16_prompt.md:338,342: N records verification; O additionally commits the
  lockfile cleanup in store, adapter and kernel, then repins those cleanup revisions
  in orchestrator (Cargo.toml:25). After removing patches, Cargo regenerated each
  downstream Cargo.lock with the committed core source. Restoring those generated
  changes did not leave clean trees, so the Git sources are retained in one O
  commit per affected repo. No source or registry dependency changed; the final
  unpatched locked build was repeated against the cleanup revisions.
- ubu-core/src/store/mutation_envelope.rs:500: `cargo fmt` incidentally reformatted
  one existing test constructor into two lines; no behavior changed. It was already
  pushed with D when noticed, so the no-force-push policy was preserved.
- P1B-16_prompt.md:294: the pre-existing Python advisory test was run through
  stdlib `runpy` instead of installing pytest; the test function passed unchanged.
- The first sandboxed unpatched build could not resolve github.com; rerunning with
  authorized UbU dependency network access passed. No additional network scope was used.

No other implementation deviations. Generated configs are removed, and the final
post-commit audit checks all sibling trees are clean and protected repository
heads unchanged.
