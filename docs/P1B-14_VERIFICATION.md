# P1B-14 verification

Kernel branch `p1b-14-static-placement` was pushed before the orchestrator pins
were changed. Both kernel packages are pinned to
`113e1cfc28478cc64f1d682ec28d4726fd3df0ae`.

## Checks

- `cargo test --workspace` in the kernel: 39 passed, 0 failed.
- `cargo test` in the orchestrator: 97 passed, 0 failed.
- All 35 existing kernel outcomes remain passing; every golden suite's byte-exact
  assertions and every existing property check pass. No golden file, existing
  golden assertion, emitted-step ordering or contract version was changed.
- All 84 existing orchestrator tests remain passing, with 13 new store-backed and
  serialization/configuration tests. P1B-9 and P1B-12 assertions are unchanged.
- `cargo run --example generate_openapi` regenerated the committed API snapshot.
  Public API additions are `occupies_capacity`, `category_tag`, `gcal_color_id` on
  scheduled steps. The previously stale snapshot also gained existing advisory
  route/schema definitions; no new advisory behavior was implemented.
- `cargo build --locked` with `.cargo/config.toml` moved aside passed.
- `cargo tree -i ubu_core` was captured with patches; `cargo tree --locked -i
  ubu_core` was captured without patches. Each has one core source; the unpatched
  graph uses committed `59f04f4` and both kernel packages at the revision above.
- Restored the patch config after the unpatched check, then removed all four
  generated sibling configs and restored generated path lockfile changes.
  All nine sibling trees were clean at the initial gate and after cleanup.
  The six prohibited code repositories and brand retained their initial revisions.

## Patched core graph

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

## Unpatched core graph

```text
ubu_core v0.1.0 (https://github.com/UbU-project/ubu-core?rev=59f04f470aa5696e6762b6b594f1a02918bc525b#59f04f47)
├── ubu_github_adapter v0.1.0 (https://github.com/UbU-project/ubu-github-adapter?rev=9ab7f7d752d22a9701250d1107f403745b795c66#9ab7f7d7)
│   └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_core v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=113e1cfc28478cc64f1d682ec28d4726fd3df0ae#113e1cfc)
│   ├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
│   └── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=113e1cfc28478cc64f1d682ec28d4726fd3df0ae#113e1cfc)
│       └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=113e1cfc28478cc64f1d682ec28d4726fd3df0ae#113e1cfc) (*)
└── ubu_store v0.1.0 (https://github.com/UbU-project/ubu-store?rev=4ed3a0a97e4ede4a57d13bd05d69688f128e3755#4ed3a0a9)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
```

## Verbatim test tails

Kernel:

```text

running 1 test
test candidate_generation::tests::candidate_bound_is_contractual ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_planning_advisory_protocol

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_planning_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_planning_cpu

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

Orchestrator:

```text
test category_has_no_tag_fallback_and_unmapped_category_is_retained ... ok
test palette_override_merges_defaults_and_matches_case_exactly ... ok
test dynamic_noncapacity_is_excluded_and_empty_kernel_is_bypassed ... ok
test calendar_contains_capacity_noncapacity_and_category_steps ... ok
test repair_carries_direct_steps_and_frozen_steps_win ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s

     Running tests/user_actions.rs (target/debug/deps/user_actions-784ad5e99b11aedf)

running 6 tests
test record_action_requires_known_schema_version ... ok
test override_records_user_override_without_transition ... ok
test complete_with_effects_but_no_universe_state_surfaces_diagnostic ... ok
test complete_records_decision_log_and_transitions_task ... ok
test complete_applies_task_effects_to_universe_state ... ok
test complete_without_effects_leaves_universe_state_unchanged ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

   Doc-tests ubu_orchestrator

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

## Verbatim calendar excerpt

From the HTTP `/calendar/current` acceptance test; the same test asserts persisted
steps equal selected candidate steps and all alternative candidates carry the
non-capacity step.

```json
{
  "steps": [
    {
      "depends_on": [],
      "end": 29685070,
      "index": 0,
      "occupies_capacity": false,
      "placement_authority": "user_override",
      "start": 29685060,
      "static_anchor": true,
      "summary": "Background routine",
      "task_id": "task_01a0c40263587553aeddb859c0c1673b"
    },
    {
      "category_tag": "commute",
      "depends_on": [],
      "end": 29685080,
      "gcal_color_id": "7",
      "index": 1,
      "occupies_capacity": true,
      "placement_authority": "user_override",
      "start": 29685060,
      "static_anchor": true,
      "summary": "Drive to work",
      "task_id": "task_01a0c40263557ab295bcc42df3c8a0e9"
    },
    {
      "depends_on": [],
      "end": 29685090,
      "index": 2,
      "occupies_capacity": true,
      "placement_authority": "planner",
      "start": 29685080,
      "static_anchor": false,
      "summary": "Dynamic work",
      "task_id": "task_01a0c40263587553aeddb870914f55dc"
    }
  ]
}
```

## Literal readings and limits

- A pair is one static conflict even if both occupancy and precedence fail. Pair
  orientation uses `(start, id)`; diagnostics then sort by
  `(first.start, first.id, second.id)`.
- Clause (b) includes non-capacity Static prerequisites. They never cause an
  occupancy conflict. The precheck consults all stored Static prerequisites,
  including those not participating; only eligible dependents participate.
- Dynamic dependency bounds apply only to eligible Static Tasks omitted for
  non-capacity or horizon reasons. Other exclusions keep prior edge-dropping
  behavior. Frozen placements still win, apart from required reindexing.
- No-capacity handling also covers an empty store or explicit empty request,
  returning the specified diagnostic without a kernel call.
- OpenAPI snapshot catch-up described above is the only incidental generated
  change. No new dependency, prohibited repository change or golden regeneration.
- Deferred unchanged: Unix-minute-zero fallback horizon, duration-estimate
  seconds/minutes mismatch, and preservation of old Static windows by kernel repair.

Complete logs, the initial revision/lockfile inventory, both `tree -I target`
outputs, the dependency graphs, and calendar excerpt are retained in the sibling
`P1B-14-results` directory.
