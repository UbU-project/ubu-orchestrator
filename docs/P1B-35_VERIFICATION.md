# P1B-35 verification

Branch in all six changed repositories: `p1b-35-occurrence-override`. All repository trees were clean before work. One signed commit per lettered section, with one section C commit in each of its three repositories. Each checkpoint passed its applicable full suite before committing.

## Landing order and pins

| Order | Repository | Published revision / checkpoint |
| --- | --- | --- |
| 1 | `ubu-schemas` | `31faf91c1e407fe1460566a133d01ffca2296917` |
| 2 | `ubu-core` | `8d8d8e4a790ed4a641029f8de2799ef129351aaf` |
| 3 | `ubu-store` | `8621bf44465b697030eedf66e38ab89354f99f85` |
| 4 | `ubu-planning-kernel` | `f50c3e9a01a2908dbfa61793189b3d04dadf9c8d` |
| 5 | `ubu-github-adapter` | `f8b189ab24374f3cd187280172c720a6aaa1a44c` |
| 6 | `ubu-orchestrator` | Section I commit containing this report; resolve with `git log -1 --format=%H -- docs/P1B-35_VERIFICATION.md`. Its final full revision is also supplied in the completion response. |

The report cannot contain its own commit hash. The preceding section H revision is `784f4e5b5e5fc26c2323f5c21ec710b7df4445ca`; section I adds only the eleven integration tests and this report. Production checkpoints: D `070a59d`, E `7fa9f14`, F `2c0fb4c`, G `fd6ecb4`. The complete orchestrator branch is pushed after section I.

The core `schemas-ref` submodule is exactly the published schema revision above. Store, kernel and adapter each pin the published core revision. Orchestrator pins that core, store and adapter and both `ubu_planning_core` and `ubu_planning_cpu` to the published kernel revision. Remote branch heads were checked with `git ls-remote` before the final downstream push.

No new dependency, external path dependency, or `[patch]` table was introduced. The kernel already uses internal workspace path dependencies; the instruction against path dependencies is read as prohibiting a local sibling checkout workaround, not requiring removal of its pre-existing workspace structure. Section C changed only `Cargo.toml` and `Cargo.lock` in each repository. No production or test source changed in those three repositories.

## Tests, warnings, formatting and transport

| Own checkout | Tests passed | Failed / ignored | Clippy before → after | Delta |
| --- | ---: | --- | --- | ---: |
| `ubu-schemas` | 2 | 0 / 0 | 0 → 0 | 0 |
| `ubu-core` | 133 | 0 / 0 | 2 → 2 | 0 |
| `ubu-store` | 100 | 0 / 0 | 0 → 0 | 0 |
| `ubu-planning-kernel` | 81 | 0 / 0 | 0 → 0 | 0 |
| `ubu-github-adapter` | 23 | 0 / 0 | 0 → 0 | 0 |
| `ubu-orchestrator` | 294 | 0 / 0 | 10 → 10 | 0 |

Schemas also validated **77 valid and 76 invalid fixtures**. Core adds three unit tests for duplicate dates, invalid dates and non-positive windows. Orchestrator has exactly eleven new integration tests: **283 + 11 = 294**. Sections D–H each passed the unchanged 283-test count. Section F updates two existing assertions for the newly supported behaviour without changing the count:

- `occurrence_drag_pins_date_without_changing_key` replaces the old routine-drag rejection expectation.
- `observed_routine_window_override_pins_without_changing_declaration` asserts that the explicit span wins even with an observed duration model, while the routine's declaration remains unchanged.

Both Clippy measurements use `cargo clippy --locked --offline --all-targets --message-format=json` in each repository's own checkout during this ticket run. Count distinct `compiler-message` warnings by `(code, message, primary spans' file, line, column)`, deduplicating repeated library/library-test messages. Counts are compared with the same method; this report does not claim unchanged source locations. Original baseline counts were retained through the interrupted session; final logs were recreated in durable workspace storage.

Full tests use `cargo test --locked --offline`, with a sanitized environment and an inherited seccomp filter denying IPv4/IPv6 socket creation. The final orchestrator run additionally uses `strace -f -qq -e trace=network`: **0 Internet-family socket mentions, 0 connect calls**, all 294 tests passing. Integration requests stay in process and Calendar uses `RecordingCalendarApi`; no real calendar or event identifiers or credentials appear in fixtures. Git and Cargo were the only network clients used for publication and dependency resolution.

Core and schemas pass `cargo fmt --check`. Only newly created orchestrator Rust files were formatted individually; the existing orchestrator/store formatting debt was not changed. OpenAPI was regenerated with `cargo run --locked --offline --example generate_openapi` and includes PUT and DELETE plus request/response schemas. An optional schema TypeScript generator/casing check was unavailable because Node dependencies were absent; no npm network access was attempted. The required Rust fixture validator passed.

## Exact lockfile deltas

Schemas and core lockfiles are byte-identical to their pre-ticket files. Core's lockfile is ignored/untracked; its pre-B byte comparison was checked before committing. The other lockfiles were parsed and compared package by package: no package, version, checksum or dependency-list changes, only the source fields below. Each old/new hash applies to both `rev=` and the source fragment after `#` on the unchanged Git URL.

| Repository | Package | Old Git revision | New Git revision |
| --- | --- | --- | --- |
| `ubu-store` | `ubu_core` | `72b3ad8d5734d30443ea62f721833212e6da5e25` | `8d8d8e4a790ed4a641029f8de2799ef129351aaf` |
| `ubu-planning-kernel` | `ubu_core` | `72b3ad8d5734d30443ea62f721833212e6da5e25` | `8d8d8e4a790ed4a641029f8de2799ef129351aaf` |
| `ubu-github-adapter` | `ubu_core` | `72b3ad8d5734d30443ea62f721833212e6da5e25` | `8d8d8e4a790ed4a641029f8de2799ef129351aaf` |
| `ubu-orchestrator` | `ubu_core` | `72b3ad8d5734d30443ea62f721833212e6da5e25` | `8d8d8e4a790ed4a641029f8de2799ef129351aaf` |
| `ubu-orchestrator` | `ubu_github_adapter` | `963d2c834aa7236ca31ea32d0acc64f0e8d82341` | `f8b189ab24374f3cd187280172c720a6aaa1a44c` |
| `ubu-orchestrator` | `ubu_planning_core` | `cf8b2ca41fdca0fb90e14e21369ae51d154c9330` | `f50c3e9a01a2908dbfa61793189b3d04dadf9c8d` |
| `ubu-orchestrator` | `ubu_planning_cpu` | `cf8b2ca41fdca0fb90e14e21369ae51d154c9330` | `f50c3e9a01a2908dbfa61793189b3d04dadf9c8d` |
| `ubu-orchestrator` | `ubu_store` | `eeacb6ef969dceaf917ff30babcb185114098a8b` | `8621bf44465b697030eedf66e38ab89354f99f85` |

The orchestrator's five entries are core, store, adapter and the kernel's two packages. An offline Cargo update initially selected an older cached semver release in the kernel; the original `1.0.28` was restored before its tests and commit. No semver delta landed.

## Verbatim synthetic evidence

These lines are copied from the final full test run. Generated synthetic Task and event IDs vary between runs.

### Test 1: occurrence key before and after

```json
{"key_after":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01/s1/2026-09-27T07:00:00/planned/t1","key_before":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01/s1/2026-09-27T07:00:00/planned/t1","window_after":{"end":"2026-09-27T15:30:00Z","start":"2026-09-27T15:00:00Z"},"window_before":{"earliest_start":"2026-09-27T07:00:00Z","latest_finish":"2026-09-27T12:00:00Z"}}
```

### Test 3: exact payload and pinned window

```json
{"duration_estimate":{"seconds":1800,"type":"fixed"},"id":"task_01a0df95dc3c75039140c9f7d0f26556","objective_id":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01","occurrence":{"key":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01/s1/2026-09-27T07:00:00/planned/t1","local_date":"2026-09-27","routine_objective_id":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01"},"provenance":{"authority_source":"system","created_at":"2026-09-27T06:00:00Z","source":{"source_id":"obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01/s1/2026-09-27T07:00:00/planned/t1","source_kind":"routine_instantiation"}},"static_window":{"end":"2026-09-27T15:45:00Z","start":"2026-09-27T15:00:00Z"},"status":"active","title":"Shower and dress"}
```

The payload has `static_window`, no `allowed_time_range`, a key retaining `/planned/`, and a declared duration of 1800 seconds. The Plan step is asserted to span 2700 seconds, exactly matching the explicit override.

### Test 5: next preview and source-marked Log

```json
{"log":{"action":"set_occurrence_override","local_date":"2026-09-27","schema_version":"ubu.orchestrator.routine_override.v1","source":{"source_id":"01a0df95dc417bc0866d8f26f6c03701","source_kind":"google_calendar"},"task_id":"task_01a0df95dc417bc0866d8f26f6c03701","window":{"end":"2026-09-27T15:30:00Z","start":"2026-09-27T15:00:00Z"}},"operations":[]}
```

The preview's verbatim operations are `[]`, both immediately and after another generate. The capture lists events once. This test also exercises a prior start fact and, in a separate synthetic state, pinning and colouring within one observed capture.

### Test 6: outside the declared range

```json
[{"code":"routine_override_outside_allowed_range","message":"Routine `obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01` on 2026-09-27: override 2026-09-27T22:00:00Z..2026-09-27T22:30:00Z is outside its declared allowed range; honoured"}]
```

### Test 7: outside the matched predecessor bounds

```json
[{"code":"routine_override_violates_after_bounds","message":"Routine `obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e02` on 2026-09-27: override 2026-09-27T09:00:00Z..2026-09-27T09:30:00Z violates matched predecessor minimum_seconds/maximum_seconds; honoured"}]
```

The test asserts the maximum-bound override remains honoured after generation, then independently checks a minimum-bound violation.

### Test 8: excluded date

```json
{"diagnostics":[{"code":"routine_override_no_occurrence","message":"Routine `obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01` does not occur on 2026-09-27"}],"error":"Routine `obj_018f3c8e9b2a7c4d8f1e2a3b4c5d8e01` does not occur on 2026-09-27"}
```

The same test rejects skipped weekdays and disabled dates through both endpoints, checks no phantom materialization, and exercises the date-loop diagnostic for a stored exception on a date with no occurrence. Test 9 checks zero/reversed spans and windows too far before/after the named date without writes, plus DELETE's nominal-restoration guard. Test 10 clears rather than re-pins the nominal window and preserves identity. Test 11 retains general occurrence edit rejection.

## Retired diagnostics and judgments

A whole-repository search including `tests/` preceded changes to the P1B-34 occurrence-drag rejection and duration-model warning. It found the production branches, documentation and historical verification, and two occurrence cases in `tests/calendar_move_resize.rs`: two rejection assertions, plus the observed-model warning assertion and lookup in the latter case. Those two tests were updated as listed above. The obsolete model warning was removed because an explicit pinned span wins without editing the duration declaration. Final searches have no references to either retired diagnostic in source, tests or documentation. Original P1B-34 JSON evidence is linked to its immutable historical revision rather than rewritten as though it had produced P1B-35 behaviour.

No disagreement with the ten judgment calls. Overrides are canonical schedule data, do not bump schedule identity, use local-date keys, emit Static windows while keeping key placement, honour/report contradictions, support Dynamic occurrence drags, enforce sanity and occurrence checks, and can be explicitly cleared.

Literal readings and implementation boundaries:

- Section 8 refers to distance from the named date. The guard is therefore the named local day plus one configured horizon on either side, rather than the one-off path's Task-creation/today-relative bound. Whole-second materialization must also have a positive span. DELETE validates the nominal restored span. Midnight resolution uses the routine timezone; dates with no resolvable midnight are rejected.
- DELETE has no request body because the ticket specifies a body only for PUT. Both return the override schema version and diagnostics.
- A future date need not already have a materialized Task. A valid rule date is sufficient; a later generate materializes the override. Existing active occurrence Tasks are refreshed through their normal admission path, retaining identity and non-window fields. Inactive history is not rewritten.
- The placement-decision Log references the Objective. It is not an execution fact on the occurrence, which would otherwise inhibit materialization. Calendar Logs also carry the Task ID and source marker. Objective, cached Task and Log admissions retain the existing separate store transaction boundaries.
- A same-capture drag and colour uses the initial Dynamic partition to preserve completion semantics after pinning. The recorded observed window is the dragged span. Across-capture ordering remains the known limit below.
- No compiler parallelism or memory caps were introduced. Reproducible orchestrator build outputs were cleaned once to reclaim disk space before the D build.

Quick UbU remains clean at `9ccc8b8e302d4e39207b749fb00bc449dc172cf0`; design remains clean at `f7c4a1dbe2979fa498ae115bb2ee74e6772d4c2c`. No real Quick UbU data was used.

## Known limits

1. **One override per date per routine.** A routine that occurs twice on one date cannot have its two occurrences overridden differently. The Phase 1b schedule subset has no such rule, so this is not reachable today.
2. **An override is a window, not a duration change.** It says when, not how long, and a span that disagrees with the routine's `duration_estimate` is honoured as given.
3. **Overrides are never garbage-collected.** An override for a past date stays on the schedule. Nothing reads it again, but it accumulates.
4. **A template edit does not revisit overrides.** Changing `nominal_start` leaves existing overrides in place, which is judgment call 3 working as intended and may still surprise.
5. **Gesture ordering across polls can diverge.** Google returns whole event state with one `updated` timestamp and no per-field history, so UbU cannot tell a drag-then-colour from a colour-then-drag within one observation. Across two captures it can: colouring, capturing, then dragging leaves the drag applying to a completed Task, which P1B-34 ignores. The same two actions before a single capture record the dragged window. This is not introduced here and is not fixed here.
6. **Overrides do not participate in `UBU-D0289` triage automatically.** The endpoint exists; nothing proposes an override when a day cannot hold its mandatory work.

