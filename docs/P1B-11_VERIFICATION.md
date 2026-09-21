# P1B-11 verification

## A–C: committed dependency alignment

All nine sibling repositories started clean. Generated Cargo patch configs were
removed before implementation; all implementation tests used committed git pins
and `CARGO_NET_OFFLINE=true`.

- Adapter A: `4b31c8c3c933a0b1cc8674a7d69849bf39484467`, pushed on
  `p1b-11-adapter-core-alignment`. Its sole Task literal now supplies `None` for
  assignee/duration_estimate and empty vectors for blocked_by/correlation_groups/tags.
- Kernel B: `951d88ded8c95ed7cf5bfe815200dea0bdffd962`, pushed on
  `p1b-11-kernel-core-alignment`. Only Cargo.toml and Cargo.lock changed.
- Orchestrator C pins both revisions, including both kernel packages. Its locked
  offline full test suite passed before conversion removal.

The kernel's `cargo test --workspace --locked` baseline and post-bump
`cargo test --workspace` produced the same 35 named passing test outcomes.
All frozen skeleton, affect, scoring/selection, rollout, and degraded-rollout
goldens matched byte-for-byte. All property checks matched, including correlation
matrix invariants, duration quantiles, bounded Wilson intervals, deterministic
seeds, and candidate bounds. No fixture, test expectation, source code, or
PLANNING_KERNEL_CONTRACT_VERSION was changed in the kernel.

## D: conversion inventory

Removed these identity conversions; the original typed values now pass directly:

| Location | Removed conversion |
| --- | --- |
| src/services/import_service.rs:156 | Adapter Task -> JSON -> current-core Task |
| src/services/import_service.rs:160 | Adapter ExternalReference -> JSON -> current-core ExternalReference |
| src/services/projection_service.rs:920 | Current-core AuthoritySource -> JSON -> adapter-core AuthoritySource |

Kept the following kernel/wire projections. None connects duplicate revisions of
a core type; pin alignment does not make the independently declared API bodies
and kernel contracts the same Rust type.

| Location / conversion | Reason retained |
| --- | --- |
| src/services/planning_service.rs:702, canonical_plan_from_payload | Reads both canonical PlanBody and older KernelPlan storage shapes; fallback remains necessary for existing payloads. |
| src/services/planning_service.rs:364, kernel_plan_body | Renames plan_id to id, renders status, adds timestamps, step indices, summaries, placement authority and metadata defaults. |
| src/services/planning_service.rs:520, persist_kernel_plan | Combines frozen/new steps, orders and indexes them, and adds admitted-plan metadata. |
| src/services/planning_service.rs:632, scheduled_task_body | Adds display titles, index and placement authority absent from ScheduledTask. |
| src/services/planning_service.rs:661, kernel_candidate_body | Sorts/indexes steps and flattens kernel scoring/probability data into the API candidate. |
| src/services/planning_service.rs:876, task_duration_estimate / duration_estimate_body | Decodes stored core duration, then maps its enum into the separate API DurationEstimateBody. This is a contract projection even where variants have equal fields. |
| src/services/planning_service.rs:905, task_correlation_groups | Decodes stored core groups, defaults missing groups to empty and maps them to separate API CorrelationGroupBody values. |
| src/services/planning_service.rs:1389, diagnostics_from_kernel | Formats kernel diagnostic codes and messages into API DiagnosticBody. |
| src/services/planning_service.rs:1553, From<TimeWindowBody> | API schema body and kernel TimeWindow remain distinct declared types, despite equal start/end fields. |
| src/services/planning_service.rs:1562, From<StaticAnchorBody> | API schema body and kernel StaticAnchor remain distinct declared types, despite equal start fields. |
| src/services/planning_service.rs:1568, repair_kernel_request | Constructs a repair candidate, propagates supersession and projects the planning request into RepairRequest. |
| src/api/planning.rs:472, From<PlanningRequestBody> | Builds TaskSpec/TaskGraph, supplies duration/value/priority/seed/schema defaults, bounds rollout budgets and converts nested API fields. |
| src/api/planning.rs:538, duration_model | Maps API duration variants to the kernel's independently validated DurationModel. |
| src/api/planning.rs:553, probability_quality_body | Converts the kernel enum to the distinct API schema enum. |
| src/api/planning.rs:562, scoring_policy | Constructs the kernel policy from the distinct API schema body. |
| src/api/planning.rs:571, candidate_role_body | Converts kernel role to the distinct API schema enum. |
| src/api/planning.rs:580, score_summary_body | Constructs the API score summary from the separate kernel response type. |
| src/api/planning.rs:590, feasibility_summary_body | Constructs the API feasibility summary from the separate kernel response type. |
| src/api/planning.rs:600, semi_legitimization_summary_body | Maps kernel summary and result enum to their API equivalents. |
| src/api/planning.rs:622, legitimization_report_body | Adds stale-affect warning and converts result, mode and dimension reports. |
| src/api/planning.rs:652, planning_mode | Converts the API mode enum to the separate kernel request enum. |
| src/api/planning.rs:659, repair_context | Wraps last_legitimate_plan_ref in Some and projects repair scope. |
| src/api/planning.rs:668, repair_scope | Collapses failed/moot/override/remaining API scopes to kernel RemainingWindow. |
| src/api/planning.rs:678, affect_profile | Projects nested tolerance bodies and API direction/mode enums into kernel types. |
| src/api/planning.rs:700, affect_observation | Projects nested observation bodies into kernel values. |
| src/api/planning.rs:719, affect_mode | Maps API mode to kernel mode. |
| src/api/planning.rs:726, affect_mode_body | Maps kernel mode to API mode. |
| src/api/planning.rs:733, affect_direction | Maps API direction to kernel direction. |
| src/api/planning.rs:740, legitimization_result_wire | Renders kernel result as the existing wire string vocabulary. |

Database and HTTP JSON encoding remains necessary persistence/wire serialization.
No test expectations were edited. No additional compilation breakage surfaced.

## Literal readings and remaining debt

- Adapter .gitignore:2 ignores Cargo.lock and none was tracked or present at
  setup. A force-added the generated lockfile to satisfy the explicit A
  `(+ Cargo.lock)` requirement. Dependency declarations were not added.
- Adapter Cargo.toml:16 and kernel Cargo.toml:23 use the full cef80fe revision,
  matching orchestrator Cargo.toml:21 and store exactly so Cargo unifies sources.
- Adapter src/candidate_mapping.rs:3 retains core::store::CandidateObject;
  src/candidate_mapping.rs:186 still constructs it. Migration to AdvisoryCandidate
  remains debt, explicitly outside this ticket.
- Kernel Cargo.toml:1 is a virtual workspace; full Rust suite means
  `cargo test --workspace`, including every workspace member and doctest.
  Property checks are existing deterministic parameter sweeps/invariants, not
  a new property-testing dependency (e.g. crates/ubu-planning-core/src/rollout.rs:557).
- Orchestrator src/api/planning.rs:472 and the table above retain projections
  between separately declared wire/kernel contracts, including equal-field
  wrappers. Replacing those public schema types would be an API redesign, not
  removal of a redundant cross-revision core conversion.
- Lettered sections E and F are recorded as separate documentation commits;
  verification outputs also live in the sibling P1B-11-results directory.

## E: committed dependency graph

`CARGO_NET_OFFLINE=true cargo tree --locked -i ubu_core`, with no patch configs:

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
└── ubu_store v0.1.0 (https://github.com/UbU-project/ubu-store?rev=51a964af132083d1123b5baed0140e5945aa3ec2#51a964af)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
```

Cargo.lock contains exactly one ubu_core package, at cef80fe; neither old core
revision remains. Adapter (23), kernel (35) and orchestrator (75) tests all pass.
The orchestrator C/D named test outcomes match exactly.
