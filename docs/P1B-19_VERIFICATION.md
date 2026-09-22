# P1B-19 verification

The only changed repository is `ubu-orchestrator`, on branch
`p1b-19-routine-occurrences`. The starting revision was `e0bf701`; implementation
and tests through section F are committed at `14a9423cda1a88e2420394e9f9f7b0e1ce29366b`.
Section G commits this report; its final pushed revision is reported in the ticket
completion response. There is one commit per lettered section.

The orchestrator and all 11 sibling repositories were clean before work and after
implementation verification. All sibling revisions are unchanged. No patch
configuration was generated, no update-all script ran, and no git pin changed.
No real Quick UbU data was read or committed. The fixture and titles are synthetic.
Compiler and test concurrency used the existing defaults, with no single-job or
single-test-thread override.

## Commits and test progression

```text
69fe705 P1B-19 A: instantiate routine schedules with explicit DST and after lowering
037b544 P1B-19 B: materialize versioned occurrences and protect execution history
d4ea1bc P1B-19 C: plan mandatory occurrences and preserve overlapping committed time
f8fd6d9 P1B-19 D: record skips and late completion and release settled occurrences in repair
439c6b2 P1B-19 E: expose read-only routine rollups and document planning semantics
14a9423 P1B-19 F: verify routine planning, history, triage, and repair end to end
```

All existing suites passed at every implementation commit: A had 143 passing
tests, B–E had 144, and F has **154 passed, zero failed**. The final count is the
138 existing tests plus five pure-instantiation tests, one materialization test,
and ten end-to-end planning tests. OpenAPI was regenerated with
`cargo run --offline --example generate_openapi` in section E.

`cargo test --offline` final verbatim tail:

```text
running 6 tests
test record_action_requires_known_schema_version ... ok
test complete_records_decision_log_and_transitions_task ... ok
test complete_without_effects_leaves_universe_state_unchanged ... ok
test complete_with_effects_but_no_universe_state_surfaces_diagnostic ... ok
test override_records_user_override_without_transition ... ok
test complete_applies_task_effects_to_universe_state ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

   Doc-tests ubu_orchestrator

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

The ten scenarios in `tests/routine_planning.rs` cover the complete routine day,
committed-time carriers and true Calendar windows, mandatory values/order,
idempotency, misses and realized floors, rollups, supersession, skip and repair,
closing windows, the horizon edge, execution conflicts, late completion, unknown
zones, held days, revival, duplicate after references, no free time, wide horizons,
all five legacy action guards, read-only rollups, and a late event inside sleep.
The final targeted suite tail is verbatim:

```text
}
P1B19_F1_END
test f10_late_event_and_sleep_keep_true_windows_and_reserve_whole_night ... ok
test f4_late_predecessor_and_closing_window_warn_without_blocking ... ok
test f3_repair_drops_skipped_occurrence ... ok
test f1_routine_day_preserves_committed_time_mandatory_order_and_idempotency ... ok
test f6_edit_conflict_late_completion_and_unknown_zone_preserve_history ... ok
test f9_wide_horizons_legacy_guards_and_read_only_rollups ... ok
test f2_missed_realized_floors_rollups_and_supersession ... ok
test f7_held_days_keep_completed_predecessor_and_reverted_lowering_revives_id ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.30s

```

## Repair regression experiment

As required by F3, the final `frozen.retain` filter in
`src/services/recalculation_service.rs:262` was temporarily removed. Running
`cargo test --offline --test routine_planning f3_repair_drops_skipped_occurrence`
then failed with exit 101: the skipped Check-in 3 remained frozen in the repaired
Plan. The original source was restored byte-for-byte before the final full suite;
the test then passed. The intentionally failing test tail is verbatim:

```text
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    f3_repair_drops_skipped_occurrence

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.09s

error: test failed, to rerun pass `--test routine_planning`
```

## Locked build and dependency audit

`cargo build --locked` passed on the clean committed implementation revision
shown above, using the committed git dependencies and no patch configuration.
Verbatim output:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.11s
```

The direct additions are exactly the approved declarations:

```toml
chrono = { version = "0.4", default-features = false, features = ["std"] }
chrono-tz = "0.9"
```

Chrono stays at the existing 0.4.45. A comparison of package name, version, and
source against `e0bf701:Cargo.lock` found exactly these eight additions, no removed
packages, and no changed existing versions or sources:

| Added package | Version |
|---|---|
| chrono-tz | 0.9.0 |
| chrono-tz-build | 0.3.0 |
| parse-zoneinfo | 0.3.1 |
| phf | 0.11.3 |
| phf_codegen | 0.11.3 |
| phf_generator | 0.11.3 |
| phf_shared | 0.11.3 |
| siphasher | 1.0.3 |

All existing Cargo.toml dependency declarations, including all five git pins,
match the starting tree. An initial offline full lockfile regeneration selected
newer existing versions; that generated lockfile was discarded before compiling.
Resolving from the original lockfile preserved every existing version and added
only the approved eight packages.

## F1 first generate: verbatim diagnostics and risk_report

The two fields below are captured directly from test F1's first HTTP generate
response, before subsequent idempotency or action checks. Generated Task IDs and
the risk report timestamp are the actual values from that run.

```json
{
  "diagnostics": [
    {
      "code": "routine_occurrence_overlaps_commitment",
      "message": "Routine occurrence `task_01a0cac70ff57a929dd8a2d43bffebaa` shares its time with commitment `task_01a0cac70fde7c61aa058dc9a53ffba1`; both stay on the Calendar and the whole span is busy"
    },
    {
      "code": "routine_occurrences_overlap",
      "message": "Routine occurrences `task_01a0cac70ff57a929dd8a28283268720`, `task_01a0cac70ff57a929dd8a299c8c9c0ac`, `task_01a0cac70ff57a929dd8a2e7a8335b96` overlap; routines are not meant to overlap and their definitions need review"
    },
    {
      "code": "routine_occurrences_overlap",
      "message": "Routine occurrences `task_01a0cac70ff57a929dd8a2ac8a06fe6d`, `task_01a0cac70ff57a929dd8a2bea0f5cf3e` overlap; routines are not meant to overlap and their definitions need review"
    }
  ],
  "risk_report": {
    "findings": [
      {
        "blocking": false,
        "category": "routine_triage",
        "detail": "Routine occurrence `task_01a0cac70ff57a929dd8a2d43bffebaa` shares its time with commitment `task_01a0cac70fde7c61aa058dc9a53ffba1`; both stay on the Calendar and the whole span is busy",
        "severity": "medium",
        "subject_ref": "task_01a0cac70ff57a929dd8a2d43bffebaa"
      },
      {
        "blocking": false,
        "category": "routine_triage",
        "detail": "Routine occurrences `task_01a0cac70ff57a929dd8a28283268720`, `task_01a0cac70ff57a929dd8a299c8c9c0ac`, `task_01a0cac70ff57a929dd8a2e7a8335b96` overlap; routines are not meant to overlap and their definitions need review",
        "severity": "medium",
        "subject_ref": "task_01a0cac70ff57a929dd8a28283268720"
      },
      {
        "blocking": false,
        "category": "routine_triage",
        "detail": "Routine occurrences `task_01a0cac70ff57a929dd8a2ac8a06fe6d`, `task_01a0cac70ff57a929dd8a2bea0f5cf3e` overlap; routines are not meant to overlap and their definitions need review",
        "severity": "medium",
        "subject_ref": "task_01a0cac70ff57a929dd8a2ac8a06fe6d"
      },
      {
        "blocking": false,
        "category": "affect_margin",
        "detail": "projected affect margin is 0.000, near or below its limit",
        "severity": "medium"
      },
      {
        "blocking": false,
        "category": "post_plan_depletion",
        "detail": "the margin-based projection ends in a depleted or at-risk band",
        "severity": "medium"
      }
    ],
    "generated_at": "2026-09-22T20:20:38.571591984Z",
    "level": "medium"
  }
}
```

## Deviations and literal-reading assumptions

- **Flagged triage deviation:** DESIGN §15.2.2 and UBU-D0289 classify an unplaceable
  mandatory occurrence as blocking. This ticket instead emits medium,
  nonblocking `routine_triage` findings for occurrence exclusions and overlaps;
  it does not mark the Calendar stale or request recalculation. Irregular
  availability is outside the Phase 1b model. See `docs/ROUTINES.md:119` and
  `src/reports/planning_analysis.rs:111`.
- **Quick UbU relative placement:** nominal-end lowering plus the completed
  predecessor's realized end replaces Quick UbU's placed-end floor. Unmatched
  references are ignored with a diagnostic rather than leaving the dependent
  unplaced. See `docs/ROUTINES.md:30`,
  `src/services/routine_instantiation.rs:214`, and
  `src/services/routine_service.rs:401`.
- **Held-day reading:** lowering uses current definitions, while Task edges and
  realized floors resolve the stored non-superseded same-day occurrence, including
  older-key completed history. See `docs/ROUTINES.md:39` and
  `src/services/routine_service.rs:324`.
- **Rollup reading:** current occurrence status, mirrored by outcome/action Logs,
  is authoritative for counts and streaks. Read-only expiry is reported as missed
  without mutation. This implements the ticket's explicit reading of UBU-D0286's
  “Logs that roll up.” See `docs/ROUTINES.md:139` and `src/api/routines.rs:45`.
- **Existing planning limits remain:** “generate succeeds” applies to the new
  committed-overlap and occurrence-triage handling, not arbitrary kernel packing
  failures or stale planned dependency cycles. The no-free-time precheck examines
  only the occurrence window against fixed intervals. No partial placement,
  mandatory kernel marker, repair risk report, or occurrence override is added.
  See `docs/ROUTINES.md:125` and `src/services/planning_service.rs:1916`.
- **Completion clock:** assertions that depend on a completion instant admit the
  completed row with the specified timestamp and current version; API completion
  continues to stamp wall-clock time. See `tests/routine_planning.rs:194`.
- **Late re-import fixture:** re-importing at 21:10Z legitimately reports the
  already-ended pinned meeting as `past_static_window`; the test helper permits
  that existing importer behavior. See `tests/routine_planning.rs:85`.
- **Lockfile workflow correction:** the discarded full-regeneration result
  described above was never built or committed; the final dependency audit is
  exact. See `Cargo.toml:10` and this report's locked-build/dependency section.

No required ticket behavior was omitted. All out-of-scope repositories, runtime
configuration, and git dependency pins are unchanged.
