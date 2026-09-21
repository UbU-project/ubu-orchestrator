# P1B-15 verification

Implemented only in `ubu-orchestrator`, on `p1b-15-planning-time`, from `0b92120`.
One commit per section A–F. No dependency or pin changes. The kernel is unchanged
at `113e1cfc28478cc64f1d682ec28d4726fd3df0ae`; core remains `59f04f4`.

## Verification

- Initial gate: all nine sibling trees clean; update-all completed up to date.
- `cargo test`: **110 passed, 0 failed**, including every one of the 97 existing
  tests and 13 added tests. Existing P1B-9, P1B-12 and P1B-14 behavioral assertions
  remain in place. Only unit expectations, timestamp fixtures and clocks changed.
- Tests cover all four unit regressions, horizon precedence/validation/default,
  actual environment overrides in isolated subprocesses, startup rejection,
  in-progress Static placement, Dynamic now bounds, repair now/frozen bounds,
  next-action filtering and stale calendars (including completed Tasks), inactive
  Static prerequisites, and UTC timestamp round trips.
- `cargo run --example generate_openapi` regenerated the schema after sections
  A, B and C. New public fields are `start_at`, `end_at` and optional `horizon`;
  `start`/`end` descriptions state Unix seconds; `stale_calendar` is documented.
- `cargo build --locked` passed with the generated config moved aside.
- Both core graphs below contain exactly one source, with unchanged git pins in
  the unpatched graph. `Cargo.toml` and `Cargo.lock` match the initial revision.
- Restored the config after the unpatched check, then removed all four generated
  sibling configs and restored generated path-lockfile changes. All nine sibling
  trees were clean after cleanup; all eight other repo revisions are unchanged.

## Verbatim cargo test tail

```text
test repair_carries_direct_steps_and_frozen_steps_win ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s

     Running tests/user_actions.rs (target/debug/deps/user_actions-784ad5e99b11aedf)

running 6 tests
test record_action_requires_known_schema_version ... ok
test complete_without_effects_leaves_universe_state_unchanged ... ok
test complete_with_effects_but_no_universe_state_surfaces_diagnostic ... ok
test complete_records_decision_log_and_transitions_task ... ok
test override_records_user_override_without_transition ... ok
test complete_applies_task_effects_to_universe_state ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s

   Doc-tests ubu_orchestrator

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

## Patched cargo tree -i ubu_core

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

## Unpatched cargo tree --locked -i ubu_core

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

## Verbatim /calendar/current step

```json
{
  "depends_on": [],
  "end": 1781082300,
  "end_at": "2026-06-10T09:05:00Z",
  "index": 0,
  "occupies_capacity": true,
  "placement_authority": "user_override",
  "start": 1781082030,
  "start_at": "2026-06-10T09:00:30Z",
  "static_anchor": true,
  "summary": "Exact static",
  "task_id": "task_01a0c42abebc7373abcc5d857ccb600a"
}
```

## Existing unit expectations changed

| Test | Location | Unit conversion |
|---|---|---|
| `store_backed_request_uses_calendar_window_and_topological_order` | `tests/planning_o9.rs:35` | Unix seconds for Calendar bounds; 15 minutes → 900 seconds |
| `store_backed_request_defaults_missing_model_to_fixed_independent` | `tests/planning_o9.rs:241` | 30-minute default → 1800 seconds |
| `stale_affect_snapshot_is_not_presented_as_current` | `tests/planning_o9.rs:504` | Observation coordinate uses Unix seconds |
| `calendar_contains_capacity_noncapacity_and_category_steps` | `tests/static_category.rs:87` | Anchors and bounds in seconds; 20 minutes → 1200 seconds |
| `absent_static_dependencies_bound_dynamic_start_or_exclude_it` | `tests/static_category.rs:285` | Dependency lower bounds use Unix seconds |
| `other_exclusions_keep_existing_dependency_behavior` | `tests/static_category.rs:326` | Horizon lower bound uses Unix seconds |

Additionally, `edge_crossings_keep_whole_statics_and_dynamic_horizon`
(`tests/static_category.rs:148`) converts units **and** strengthens precision:
14:50:12 stays 14:50:12, 17:10:01 stays 17:10:01, and 15:10:00.001 becomes integer
Unix second 15:10:00, without the former minute floor/ceil expansion.

The timestamp fields added to `tests/next_action.rs:324` and
`tests/static_category.rs:571` are schema fixture updates, not weakened assertions.
Fixed clocks in `tests/planning_o9.rs:836`, `tests/static_category.rs:26` and
`tests/next_action.rs:288` preserve the intent of historical Calendar fixtures.

## Literal readings and deviations

- To satisfy “Dynamic Tasks never start before now,” Dynamic per-task starts are
  `max(H.start, now, absent-prerequisite end)` even for an explicit/stored scope
  beginning in the past. Scope/Static participation is otherwise preserved.
  See `src/services/planning_service.rs:574` and `docs/PLANNING_TIME.md:22`.
  A scope with no future room may still produce the unchanged kernel failure.
- All derived coordinates are integer `unix_timestamp()` values. Fractional
  seconds are discarded, and explicit horizons must remain ordered after that
  conversion (`src/services/planning_service.rs:857`, `docs/PLANNING_TIME.md:8`).
- Full request coordinates pass through unchanged, while timestamp labels
  interpret them as seconds. Unrepresentable RFC 3339 coordinates return
  `invalid_schedule_timestamp` (`src/planning_time.rs:6`,
  `docs/PLANNING_TIME.md:13`). No dependency was added for formatting.
- Legacy scalar durations retain a positive floor, applied in seconds; legacy
  minute multiplication uses saturating arithmetic (`src/services/planning_service.rs:973`).
- Active prerequisites mean exactly the Tasks returned by `query_active_tasks`,
  currently status `active` (`src/services/planning_service.rs:1563`).
- Failed enforce-mode legitimization keeps its existing diagnostic precedence.
  Otherwise an all-ended Calendar returns `stale_calendar` even if all its Tasks
  have completed (`src/services/next_action_service.rs:24`).
- Current category documentation was updated to remove the now-fixed P1B-14 time
  debts (`docs/CATEGORY_COLOUR.md:40`). Historical verification records are unchanged.
- No other deviations. The unchanged kernel may reject an entire oversized backlog;
  `ubu-ui` still renders minutes until it adopts timestamp fields. Both limitations
  are recorded in `docs/PLANNING_TIME.md:31` and `docs/PLANNING_TIME.md:36`.

The sibling `P1B-15-results` directory retains full logs, baseline inventory,
`tree -I target`, both dependency graphs, the calendar step and final clean status.
