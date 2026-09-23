# P1B-19a verification

Only `ubu-orchestrator` changed, from `80cc870`, on branch
`p1b-19a-routine-overlap-gate`. Sections A–F are committed at
`2257fa23b75ca27e63f09c811f0448d86979b6ef`. Section G commits this report; the final pushed revision
is reported in the completion response. There is one commit per lettered section.

All 12 sibling working trees were clean before work and after implementation
verification. The other 11 repositories retain their original revisions. No
patch configuration was generated and no update-all script ran. Cargo.toml,
Cargo.lock, every git pin, and the historical P1B-19 verification record are
unchanged. All fixtures and test inputs are synthetic; no test reads a real Quick
UbU file. Compiler/test concurrency used the existing defaults without overrides.

## Commits and tests

```text
9913232 P1B-19a A: support grouped diagnostic rejections
ad3048f P1B-19a B: detect static routine overlaps with local-time DST classification
70ff925 P1B-19a C: share live routine loading without acquiring locks
71ab6a5 P1B-19a D: reject overlapping prospective live routines before any import writes
17b60ed P1B-19a E: select the first actionable Calendar placement without replanning
2257fa2 P1B-19a F: verify overlap rejection, planning fallback, and settled placement selection
```

All existing suites passed at every commit: A had 154 passing tests, B–E had 158,
and F has **164 passed, zero failed**. Four new pure tests cover the six specified
overlap scenarios plus already-lowered local starts. Six new integration tests
cover atomic rejection and dry runs, grouped reporting, legal overlaps, stale live
routines and Objective-ID replacement, inactive mainline divergence, DST-only
exemption, direct-admission planning safety, and settled Calendar placements.

The grouped-rejection test also covers a seven-routine clique (21 pairs, one
group), self-overlap wording both alone and inside a larger component, and the
25-group reporting bound. The next-action test checks that Plan payloads, Log
counts, and mutation counts do not change; it also checks distinct blocked IDs
and five ordered samples when a Plan contains duplicate or missing Task IDs.

Verbatim `cargo test --offline --locked` final tail:

```text
running 6 tests
test record_action_requires_known_schema_version ... ok
test complete_records_decision_log_and_transitions_task ... ok
test override_records_user_override_without_transition ... ok
test complete_without_effects_leaves_universe_state_unchanged ... ok
test complete_applies_task_effects_to_universe_state ... ok
test complete_with_effects_but_no_universe_state_surfaces_diagnostic ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

   Doc-tests ubu_orchestrator

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

```

Verbatim final focused next-action tail (after removing unrelated formatting
changes from the test file):

```text

running 8 tests
test next_action_requires_known_schema_version ... ok
test next_action_blocks_failed_enforce_calendar ... ok
test next_action_calendar_equal_placements_tiebreak_by_task_id ... ok
test next_action_selects_first_legitimized_calendar_placement ... ok
test next_action_selects_ready_task_by_priority_and_explains_it ... ok
test next_action_returns_bounded_diagnostic_when_all_tasks_are_blocked ... ok
test next_action_warn_only_calendar_recommends_and_surfaces_warning ... ok
test calendar_steps_over_completed_and_skipped_tasks_without_writing ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s

```

## Verbatim HTTP 400 rejection body

Captured between `P1B19A_REJECTION_BEGIN` and `P1B19A_REJECTION_END` in
`tests/quick_ubu_import.rs:351`. The objects and mutation-envelope counts were
unchanged after both the real and dry-run rejection.

```json
{
  "diagnostics": [
    {
      "code": "overlapping_routines",
      "message": "Routines overlap each other, first 2026-09-22: `00000000-0000-4000-8000-000000000041` (Morning A) 07:00:00-07:30:00; `00000000-0000-4000-8000-000000000042` (Morning B) 07:15:00-07:25:00 (1 pair, up to 366 dates in the next year)"
    }
  ],
  "error": "1 overlapping routine pair in 1 group; nothing was imported. Routines must not overlap: stagger their start times, shorten one, or make one transparent."
}
```

## Unchanged lockfile and locked build

The lockfile was compared byte-for-byte with `80cc870:Cargo.lock`; its blob and
SHA-256 evidence are:

```text
baseline blob: 41a12475146af369156a79e49fb0ab9b724cd4db
current blob:  41a12475146af369156a79e49fb0ab9b724cd4db
SHA-256: b62231db8e3215a08f5cc4d72191cf9bbbbc8ae3ba8ecf9e34f4eccd19a07368
git diff 80cc870 -- Cargo.lock: empty
```

`cargo build --locked` passed with no patch configuration on the clean committed
A–F revision above. Verbatim output:

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.11s
```

## Deviations and literal-reading assumptions

- **Commit sequencing:** F's prescribed fixture corrections and F1 expectation
  updates land in D with the import gate. Otherwise D would intentionally reject
  the old fixtures and violate the requirement for green tests at every commit.
  F contains the new regression tests. The one-commit-per-letter policy is
  preserved. See `fixtures/quick-ubu/snapshot-small.json:81`,
  `fixtures/routines/snapshot-day.json:119`, and `tests/routine_planning.rs:301`.
- **Year and start filtering:** the check covers 366 UTC days from planning now,
  and checks only occurrences starting inside that span. It does not extend the
  span to cover later enablement windows or include starts preceding now. See
  `src/services/quick_ubu_import.rs:692`,
  `src/services/routine_instantiation.rs:354`, and `docs/QUICK_UBU_IMPORT.md:79`.
- **DST interpretation:** parsed chrono-tz zones must be equal; nominal local
  intervals use the already-lowered start plus scalar duration. Cross-zone pairs
  never receive the exemption. Unknown-zone and unevaluable definitions do not
  reject, and instantiation diagnostics are discarded by this gate. See
  `src/services/routine_instantiation.rs:321` and
  `src/services/quick_ubu_import.rs:650`.
- **Bounded date bookkeeping:** each pair keeps only its latest counted local
  date. Revisited older open instances of a long-duration routine are not counted
  again. No per-date set is retained. See
  `src/services/routine_instantiation.rs:392`.
- **Representative group windows:** a member's first pair in the sorted overlap
  results supplies its displayed local window; members are then sorted by window
  and Objective ID. At most 25 group messages are returned, with a separate final
  omitted-group-count diagnostic when necessary. See
  `src/services/quick_ubu_import.rs:745` and `:798`.
- **Defensive civil-time bounds:** an extreme duration outside chrono's
  representable civil range cannot earn a DST exemption; an unrepresentable
  window endpoint displays `out-of-range`. Ordinary endpoints use HH:MM:SS.
  See `src/services/routine_instantiation.rs:309` and `:321`.
- **Explanation wording:** the selection rule remains
  `legitimized_calendar_first_placement`, but the explanation now says “selected
  the first actionable Task placement in the current legitimized Calendar.”
  The pinned assertion is updated. See `src/services/next_action_service.rs:506`
  and `tests/next_action.rs:124`.
- **Existing next-action guards remain:** failed legitimization, an entirely
  elapsed nonempty Calendar, and no admitted/active Tasks retain their existing
  early responses. Only the current-Calendar selection branch changes; the
  readiness path and all mutation behavior are unchanged. See
  `src/services/next_action_service.rs:29`, `:48`, `:66`, and `:93`.

## Historical record

`docs/P1B-19_VERIFICATION.md` records F1's earlier output with the intentionally
illegal overlapping routine fixture. The legal fixture and updated F1 assertions
in this ticket supersede that example. The historical record is not edited.
The new direct-admission test in `tests/routine_planning.rs:732` preserves coverage
of three overlapping routine occurrences whose union outlasts the carrier.
Planning's overlap and triage implementation is unchanged.
