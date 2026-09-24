# P1B-26 verification

## Landing order and revisions

All twelve repositories were clean at the ticket's expected starting revisions.
Part 1 landed first, touching only `ubu-design`, with separate A–E commits and
no device work or runtime implementation. Its pushed revision is
`f7c4a1dbe2979fa498ae115bb2ee74e6772d4c2c`; see
[Part 1 verification](../../ubu-design/docs/P1B-26_PART1_VERIFICATION.md).
It adds UBU-D0290 and UBU-Q0160. Existing baseline 0.1 and Phase 1b 0.2 contract
version declarations were preserved without a bump; the prompt's 0.1-only claim
was stale. The requested mobile subsection took §16.10.6, with the existing
correlation and solver subsections mechanically renumbered to .7 and .8.

All eight changed repositories use `p1b-26-routine-effects`. Part 2's seven
repositories landed in dependency order, with Quick UbU independent:

| Repository | Pushed revision / section |
| --- | --- |
| ubu-schemas | `005efffe45f81ad190c392e28462808d25bef5d3` (F) |
| ubu-core | `867c7a95edcba452470604c8fbce33b2d96986c1` (F) |
| quick-ubu, independent | `9ccc8b8e302d4e39207b749fb00bc449dc172cf0` (H) |
| ubu-store | `f5831d5725d2f781d3221a0885257d0b4fbe1cf2` (F) |
| ubu-planning-kernel | `95c0e05d11ebf12e5e8717591818eb7670dca5e1` (F) |
| ubu-github-adapter | `c9cae5016fd5ec2a469f0e22f605d1c8c1960f1e` (F) |
| ubu-orchestrator | the K commit on `p1b-26-routine-effects` containing this report |

Core's `schemas-ref` gitlink matches the pushed schema revision. Each consumer
pins that exact pushed core revision. Orchestrator pins that same core plus the
exact three pushed consumer revisions; both planning crates use the kernel
revision above. Its separate commits are F `57d6b5b`, H `5a84d04`, I `3677e76`,
and J `2e2e098e91f8a11ca4b78ef40541c77738ec26bb`. K contains only the six-test file
and this report; its final SHA is in the final ticket response rather than
attempting to put a commit's own hash inside it. There was no G source fix and
therefore no artificial empty G commit. Each changed letter has its own commit
in each affected repository, after a green full suite, with a clean tree after
commit. No force pushes were used.

The first schema push received a transient GitHub Internal Server Error; retry
succeeded. Quick UbU's SSH push encountered an authentication failure; an HTTPS
retry succeeded. No dependency was pinned to an unpushed revision. Four unchanged
siblings (UI, brand, devshell and model-committee) retain their starting heads
and clean trees. Only synthetic fixtures were read or committed for testing.

## Tests, Clippy and generated output

| Repository | Passing tests | Clippy before → after | Delta |
| --- | ---: | ---: | ---: |
| ubu-design | documentation checks; no executable suite | N/A | N/A |
| ubu-schemas | 1; 75 valid / 73 invalid fixtures | 0 → 0 | 0 |
| ubu-core | 129 | 2 → 2 | 0 |
| ubu-store | 100 | 0 → 0 | 0 |
| ubu-planning-kernel | 81 | 0 → 0 | 0 |
| ubu-github-adapter | 23 | 0 → 0 | 0 |
| quick-ubu | 445 | 16 → 16 | 0 |
| ubu-orchestrator | 204 | 10 → 10 | 0 |

Schema fixtures passed `cargo run -p validate-fixtures`. Schema, the three
consumers and Quick UbU ran `cargo test --workspace`; core and orchestrator ran
`cargo test --all-targets` plus `cargo test --doc`. Counts sum the passed numbers
on each `test result:` line in one full suite, excluding the extra focused run
of the six new tests. No tests failed or were ignored in the final suites.
Orchestrator F passed all 198 tests **before behavior changes**; H, I and J also
passed 198, and K adds exactly six for 204. Core expands existing fixture and
omission checks; Quick UbU extends its existing round-trip test, so their test
counts do not increase. Ordinary Cargo/test concurrency was retained, without
compiler or execution caps.

Clippy was measured with `cargo clippy --all-targets` on every Rust repository's
own checkout before Part 2 edits and after its changes, during this ticket run.
Toolchain: rustc 1.98.1 / clippy 0.1.98. Counts are emitted diagnostic `warning:`
headers, excluding Cargo's `generated N warnings` package summaries and duplicate
target passes. Different locations with identical messages count separately.
The complete warning-message multisets match before/after, not only the totals;
no earlier ticket's quoted count was used as a baseline.

`cargo run --example generate_openapi` succeeded. **OpenAPI is byte-unchanged.**
No API response shape changed. Schema and core remain fmt-clean. Only the new
orchestrator test file was formatted with rustfmt; no repository-wide formatting
was applied to the three repositories with existing divergence. `git diff
--check` passes. New core assertions were written to maintain its clean format.

## Exact lockfile and dependency checks

- Design has no Cargo manifest or lockfile.
- Schema, core and Quick UbU lockfiles are byte-identical to saved pre-ticket
  bytes; their manifests are unchanged. Core's ignored on-disk lockfile was
  compared too.
- Store, kernel and adapter each change only the `ubu_core` git source in the
  lockfile and its corresponding manifest revision.
- Orchestrator changes only the git sources of `ubu_core`, `ubu_store`,
  `ubu_planning_core`, `ubu_planning_cpu` and `ubu_github_adapter`, plus the same
  five manifest pins. Each requested `cargo update -p` was run.

Package lists, versions, checksums and dependency arrays are identical after
excluding those exact git-source entries. There is no new dependency, path
override or `[patch]` table. Existing Quick UbU/kernel workspace path relations
are unchanged. Quick UbU's scheduler and the orchestrator's planner/precondition
gate and instantiator are unchanged; occurrence payload lowering is in
`routine_service.rs`.

## Verbatim occurrence fields from test 1

`imported_declarations_lower_to_durable_occurrence_fields` uses
`POST /import/quick-ubu`, generates the day, then reads the stored Brush and Sleep
occurrence payloads. It verifies the fields survive another materialization:

```text
P1B26_EFFECTS {"mutations":[{"operation":"set_fact","payload":true,"target":"facts.teeth_clean"}]}
P1B26_PRECONDITIONS {"expected":true,"predicate":"equals","target":"facts.teeth_clean"}
```

## Verbatim cold-store evidence from test 2

`cold_store_seeds_on_completion_and_then_plans_the_consumer` confirms the first
Plan contains only Brush and floss and names Sleep's precondition in
`blocked_tasks`, with no persisted UniverseState. Completion produces:

```text
P1B26_BEFORE facts=(no UniverseState)
P1B26_COMPLETE diagnostics=[]
P1B26_AFTER facts={"teeth_clean":true}
```

The test also checks one UniverseState object, the completion's effective capture
time, ordinary user provenance, and a compartment label matching the completing
Task. The next generated Plan has status `ok`, contains only Sleep, and has no
blocked Tasks. This exercises the real admission/persistence path, not a manually
preseeded state or a planner mock.

## Verbatim re-blocking evidence from test 3

A mainline routine template declares a `clear_fact` mutation. Its occurrence
carries that mutation; completing it after brushing removes the fact. Sleep,
previously eligible, is then named in `blocked_tasks` on the next generation:

```text
P1B26_REBLOCKED {"precondition":{"expected":true,"predicate":"equals","target":"facts.teeth_clean"},"task_id":"task_01a0d3e7509a7f0386d98095d1847298"}
```

The Task id is generated by this synthetic test run. The assertion verifies the
actual consumer id and full leaf, and that the consumer is not in
`unplaced_tasks`. Tests 4–6 cover absent/false verification preserving P1B-25
ordering, one bare leaf versus multiple `all_of` leaves including an unresolved
requirement, and invalid establishment targets skipping only their mutations
while the routine still imports.

## Authorized test updates and section G

Section G required **no changes**. Store, kernel and adapter passed against the
new core with only manifest/lockfile repins; no production or test source changed
in any of them. No constructor or dropped-`Eq` dependency surfaced.

Preflight found three existing tests whose expectations necessarily change.
Work stopped under the ticket's explicit rule, and the user authorized each:

1. `ubu-orchestrator/tests/user_actions.rs:219`: the cold-store completion test
   formerly expected `task_effect_universe_state_absent`. It is renamed to describe
   seeding and now asserts empty diagnostics plus `facts.task_outcome = "done"`.
   The user's approval was: “I approve updating the test at
   `ubu-orchestrator/tests/user_actions.rs:219`. Continue.”
2. `ubu-orchestrator/tests/declared_requirements.rs`: the four-namespace fixture
   now expects exactly three `establishes_invalid_target` diagnostics. Its
   original resolution, sorting and tightest-bound assertions remain intact.
3. `quick-ubu/cli/tests/commands.rs`: the declaration round-trip test includes
   default `verify: false` in one expected requirement and explicitly authored
   `verify: true` in the other, proving both survive serialization.

For the latter two the user said: “I approve updating these 2 tests. Continue.”
These are the three approved exceptions; they do not broaden the consumer scope.
The explicitly requested log-service unit-test rewrite also admits its synthetic
Task, uses the admitted version, and checks that the intrinsic-affect increment
applied. The organization-mode sibling is unchanged.

## Absent-state diagnostic

`task_effect_universe_state_absent` had exactly one production emitter, the
log-service early return. It was removed. No production or test reference remains;
documentation retains the historical probe. Valid nonempty permitted effects
now seed first, then apply through the existing version-checked store path.
No-effects/empty-effects and mode-rejected branches still short-circuit before
seeding. Failure to reread an admitted seed is an internal error, not a successful
completion response silently discarding its effects.

## Judgment calls and literal readings

No disagreement with the seven behavioral choices: durable template fields,
opt-in verification, facts-true establishment syntax, first-use seeding, dropping
only the two specified `Eq` derives, reuse of the reactive gate, and `all_of` for
multiple leaves. The following interpretations and limits are explicit:

- Task effects are inline in `task.schema.json`, not a standalone effect schema.
  The template therefore references its existing `#/properties/effects` fragment;
  preconditions reference the same existing precondition schema as Task. No
  duplicated effect contract or dependency was introduced.
- Core's existing Task precondition type accepts predicate names as strings and
  diagnoses invalid names during evaluation. The core fixture test checks that
  existing evaluator behavior; the schema fixture rejects the invalid predicate
  structurally. No new validator was invented for routine templates.
- “Skipped establishment” skips the proposed `set_fact` mutation, not the routine
  or P1B-25 ordering declaration. Thus the approved four-namespace test still
  resolves its ordinary edges while reporting that non-fact declarations cannot
  produce these effects. Malformed `facts.` targets also receive the same reason.
- Every verified requirement contributes its leaf even if edge resolution fails.
  No leaves are silently discarded for missing establishers, ambiguous ordering
  or invalid bounds; invalid preconditions remain subject to the existing gate.
- Test 3 authors its destroyer through a mainline template. Quick UbU's
  `establishes` has no destroyer syntax, and adding one would exceed this ticket.
- The effects seed is created only for nonempty, mode-permitted effects when no
  state exists, using ordinary admission and reread as requested. No bootstrap
  repository is required. Planner reads still synthesize an empty unpersisted
  state until the first such completion.
- Reactive checking reports truth at generation time; it cannot guarantee that
  later real-world changes leave an already generated Plan valid. The ticket's
  “never silently wrong” framing is scoped to that current-state check, not a
  proof about execution-time state. For the generated `equals true` leaves,
  missing targets block; the existing generic `absent` predicate is not redefined
  by the wording of known limit 5.

## Known limits (verbatim)

1. **Reactive, not predictive.** A precondition is checked against the world as it is when the Plan is built, not as it will be when the Task runs. A consumer whose establisher is planned for later the same day is blocked now and becomes eligible only after that establisher is actually completed. This is conservative by construction and is the opposite failure mode from plan-state simulation.
2. **`establishes` can only set facts true.** Numeric, set-membership and event-marker effects, and facts with values other than `true`, need a richer declaration than a list of target names.
3. **`success_probability` is still read nowhere.** A probabilistic effect applies in full on completion, exactly as it did for one-off Tasks. The field remains declared and unimplemented.
4. **A blocked mandatory occurrence is not `unplaced_tasks`.** It is reported in `blocked_tasks` with its precondition, which is the right diagnostic, but `UBU-D0289`'s partial-placement machinery never sees it and a caller reading only `unplaced_tasks` will not know the day is short one routine.
5. **The seed is empty.** A store that never bootstrapped gets a `UniverseState` with no facts, so every precondition over a target nothing has established yet is false rather than unknown. There is no distinction between "false" and "never observed."
