# P1B-17 verification

Implemented chunked CPU sweep and default orchestrator strategy selection on
`p1b-17-chunked-sweep` in both repositories. Kernel revision:
`8e22b57c1d85d0a3ed179c76e8382cec3aba3d35` (pushed before the orchestrator pin bump).
The orchestrator manifest and committed lockfile pin both kernel packages to this
revision. `ubu_core` remains `358086dba4575984beccdaa7f5cc0d457a276fe9`.
No dependency was added or otherwise repinned.

All 11 sibling trees were clean before work. The devshell update found every
repository already up to date. Generated configs were removed after verification;
Cargo.lock source entries changed by local patching were restored. All sibling
trees were checked clean before the verification document was added; the final
section H commit contains this document only.

## Tests

Kernel: 56 tests pass, including all 39 pre-existing tests. Every kernel golden
matches unchanged. Greedy `CpuStrategy`, candidate generation (apart from the
required `proposal_key` visibility), rollout semantics, existing test files, and
fixture bytes are unchanged. Section A alone also passed the complete 39-test
baseline. `cargo clippy --workspace --all-targets` completed without warnings.

Orchestrator: 130 tests pass, including all 127 pre-existing tests. The P1B-9,
P1B-12, P1B-14, P1B-15, and P1B-16 assertions remain intact, including
`ranked_candidates_are_persisted_and_calendar_selects_rank_one`.
Configuration tests exercise default chunked, explicit chunked/greedy, and startup
rejection naming `UBU_PLANNER_STRATEGY` in both state constructors. Recalculation
rescues the changed-priority request under chunked and fails under greedy.

Property comparison: 3000 deterministic LCG cases; 203 sweep-only solutions;
0 greedy-only solutions. Every candidate was hard-valid, was not RejectObvious,
and placed every Task exactly once. Whenever greedy succeeded, the candidate
set's best utility was at least the greedy baseline utility (tolerance 1e-12 for
floating summation order). These tests run in the normal suite.

### Kernel: verbatim pass/fail lines

```text
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.66s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### Kernel: verbatim final tail

```text
   Doc-tests ubu_planning_core

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

   Doc-tests ubu_planning_cpu

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

### Orchestrator: verbatim pass/fail lines

```text
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### Orchestrator: verbatim final tail

```text
test complete_without_effects_leaves_universe_state_unchanged ... ok
test complete_applies_task_effects_to_universe_state ... ok
test complete_records_decision_log_and_transitions_task ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

   Doc-tests ubu_orchestrator

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

## Dependency verification

`cargo tree -i ubu_core` with generated patches (one path source):

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

With `.cargo/config.toml` moved aside, the committed git-source lockfile restored,
`cargo build --locked` passed. The config was restored as requested, then removed
as part of final generated-config cleanup. `cargo tree -i ubu_core --locked`:

```text
ubu_core v0.1.0 (https://github.com/UbU-project/ubu-core?rev=358086dba4575984beccdaa7f5cc0d457a276fe9#358086db)
├── ubu_github_adapter v0.1.0 (https://github.com/UbU-project/ubu-github-adapter?rev=5664000e823aee6f35439d4ef15f218285768bb4#5664000e)
│   └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_core v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=8e22b57c1d85d0a3ed179c76e8382cec3aba3d35#8e22b57c)
│   ├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
│   └── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=8e22b57c1d85d0a3ed179c76e8382cec3aba3d35#8e22b57c)
│       └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
├── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=8e22b57c1d85d0a3ed179c76e8382cec3aba3d35#8e22b57c) (*)
└── ubu_store v0.1.0 (https://github.com/UbU-project/ubu-store?rev=685b68e19d57014f7be355993286562644a80a2d#685b68e1)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
```

Both kernel package sources were additionally verified individually:

```text
ubu_planning_core v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=8e22b57c1d85d0a3ed179c76e8382cec3aba3d35#8e22b57c)
├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
└── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=8e22b57c1d85d0a3ed179c76e8382cec3aba3d35#8e22b57c)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=8e22b57c1d85d0a3ed179c76e8382cec3aba3d35#8e22b57c)
└── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
```

## Verbatim orchestrator rescue output

### chunked

```json
{
  "diagnostics": [],
  "steps": [
    {
      "depends_on": [],
      "end": 1781082600,
      "end_at": "2026-06-10T09:10:00Z",
      "index": 0,
      "occupies_capacity": true,
      "placement_authority": "planner",
      "start": 1781082000,
      "start_at": "2026-06-10T09:00:00Z",
      "static_anchor": false,
      "summary": "Task 1",
      "task_id": "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e01"
    },
    {
      "depends_on": [],
      "end": 1781083200,
      "end_at": "2026-06-10T09:20:00Z",
      "index": 1,
      "occupies_capacity": true,
      "placement_authority": "planner",
      "start": 1781082600,
      "start_at": "2026-06-10T09:10:00Z",
      "static_anchor": false,
      "summary": "Task 2",
      "task_id": "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e02"
    }
  ]
}
```

### greedy

```json
{
  "diagnostics": [
    {
      "code": "SkeletonFailure",
      "message": "Could not build deterministic skeleton for task task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e01: insufficient available window for deterministic skeleton placement"
    }
  ],
  "steps": null
}
```

## Deviations and literal-reading assumptions

No functional or scope deviations.

- `P1B-17_prompt.md:10–17` explicitly requires the devshell scripts, so their
  temporary generated configs were treated as authorized workflow artifacts
  despite the no-other-repo-change rule at `P1B-17_prompt.md:26`. No persistent
  sibling changes remain.
- `P1B-17_prompt.md:275–279`: path patching removes git source entries from Cargo
  lockfiles. The committed sources were restored before the unpatched locked
  build and again after final cleanup. Only the two requested kernel pins changed.
- `P1B-17_prompt.md:34`: the pre-existing `static_placement.rs` formatting issue
  was left untouched. New files and changed CLI/configuration files were formatted;
  section A retains existing formatting around the extraction and visibility change
  so the legacy candidate generator otherwise stays byte-for-byte identical.
- `P1B-17_prompt.md:275–279`: section H is committed in the orchestrator, which
  owns the requested verification document. It needs no additional kernel commit.
- `crates/ubu-planning-core/tests/chunked_sweep.rs:396`: the property utility
  comparison allows 1e-12 for floating summation order when identical placements
  appear in different step orders. Exact placement equality controls deduplication.

## Repository trees

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
│   ├── PLANNING_PRIORITY.md
│   └── PLANNING_TIME.md
├── examples
│   ├── generate_openapi.rs
│   └── run_fixture_loop.rs
├── fixtures
│   ├── fixture-loop
│   │   ├── expected-next-action.json
│   │   └── github-small.json
│   └── github
│       └── issues-small.json
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
    ├── ranges_priority.rs
    ├── reports.rs
    ├── static_category.rs
    ├── support
    │   └── advisory_fixture.rs
    └── user_actions.rs

14 directories, 92 files
```
