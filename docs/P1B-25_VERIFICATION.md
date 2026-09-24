# P1B-25 verification

## Revisions and scope

Both changed repositories and all ten read-only siblings had clean working
trees before work. Starting revisions:

| Repository | Starting revision |
| --- | --- |
| quick-ubu | `1492a0c29308c1bb25c789d6650ce605e3694f4f` |
| ubu-orchestrator | `8f003d181ef54a7b9ceba9797aef942d111fdb27` |

Both use branch `p1b-25-declared-requirements`, with one commit per section:

| Section | Repository | Revision |
| --- | --- | --- |
| A | quick-ubu | `c83fe12429f03d6b209112a2a45c6cf578e1a99c` |
| B | ubu-orchestrator | `e42b72d` |
| C | ubu-orchestrator | `1df5bd6` |
| D | ubu-orchestrator | `8b2b800` |
| E | ubu-orchestrator | `6702946913d4d88d0a9d3f55b80517893167c3c9` |
| F | ubu-orchestrator | the commit containing this report and the six-test file |

Quick UbU A was tested, committed and pushed before implementing orchestrator B
or C. The final orchestrator F SHA is supplied in the ticket's final response;
its own hash cannot be embedded in this report. Production behavior and OpenAPI
were complete at D; E and F add documentation and tests only. Each lettered
commit followed a green full suite and left its working tree clean. No force
push was used.

No pin moved, no dependency was added, and both `Cargo.lock` files are
**byte-identical** to saved pre-ticket bytes. Both manifests are byte-identical
too. The ten read-only siblings retain their original revisions and clean
working trees; all saved sibling manifests/lockfiles also match. In particular,
core remains `e022677`, schemas `5c134ad`, store `6fa8826`, kernel `6595bfe`,
adapter `e47d5bf`, and design `1a64e6b`. Quick UbU's `core/src/planning.rs` and
all downstream planner/instantiator source are unchanged. Only synthetic
fixtures were used; no real Quick UbU data was read or committed.

## Tests, warnings and generated schema

| Repository | Passing tests | Clippy warnings before → after | Delta |
| --- | ---: | ---: | ---: |
| quick-ubu | 445 (one new round-trip test) | 16 → 16 | 0 |
| ubu-orchestrator | 198 (six new HTTP tests, from 192) | 10 → 10 | 0 |

Quick UbU ran `cargo test --workspace`. Orchestrator ran
`cargo test --all-targets` and `cargo test --doc`; sections B through E each
passed 192, and F passed 198. Counts sum the passed numbers on each successful
`test result:` line in one complete suite, without adding the separate focused
six-test run again. There were no failed or ignored tests. Normal Cargo and
test concurrency were used; no compiler or execution caps were introduced.

Clippy was measured using `cargo clippy --all-targets` on each repository's own
checkout before edits and after completion, in this ticket run. Toolchain:
`rustc 1.98.1 (48a229cea 2026-09-01)`, `clippy 0.1.98 (48a229ceae 2026-09-01)`.
The count is diagnostic `warning:` headers, excluding Cargo's
``warning: `package` ... generated ...`` summaries; Cargo's duplicate target
diagnostics are not added again. Distinct locations count separately even
when messages match. The full warning-message multisets match before/after,
not just the totals; no previous ticket's warning count was used as a baseline.

`cargo run --example generate_openapi` succeeded. The generated schema adds
`QuickUbuResolved` and the required array field `QuickUbuImportResponse.resolved`.
Only the newly created Rust test file was formatted, with
`rustfmt --edition 2021 tests/declared_requirements.rs`. Existing formatting
divergence was not reformatted. `git diff --check` passes.

## Verbatim test 1 evidence

The test reads stored `routine_instance_template.after` edges and looks up
their predecessor titles. It prints:

```text
Routine | Establisher | Bounds (seconds)
Brush morning | Breakfast | [0..1800]
Brush night | Dinner | [0..10800]
```

The response's `resolved` list, verbatim from the same test:

```json
[
  {
    "establisher_quick_ubu_id": "00000000-0000-4000-8000-000000000001",
    "establisher_title": "Breakfast",
    "quick_ubu_id": "00000000-0000-4000-8000-000000000004",
    "target": "facts.fed"
  },
  {
    "establisher_quick_ubu_id": "00000000-0000-4000-8000-000000000003",
    "establisher_title": "Dinner",
    "quick_ubu_id": "00000000-0000-4000-8000-000000000005",
    "target": "facts.fed"
  }
]
```

Both bounds default their omitted offset to zero. Reimport is unchanged and
returns the same resolutions. Another subcase proves selection uses the latest
end rather than the latest start, accepts an end exactly at the requiring start,
ignores later establishers, and ignores a tie at an earlier nominal end.

## Verbatim test 2 plan

The synthetic day includes all three meals, both brushes, and Sleep. Only the
night brush declares `facts.teeth_clean`; Sleep resolves to it. The full
`POST /planning/generate` response has status `ok`, six ordered steps and no
unplaced Tasks:

```text
PROBE[plan] status="ok"
PROBE[plan] 2026-09-24T08:00:00Z Breakfast
PROBE[plan] 2026-09-24T08:15:00Z Brush morning
PROBE[plan] 2026-09-24T13:00:00Z Lunch
PROBE[plan] 2026-09-24T19:00:00Z Dinner
PROBE[plan] 2026-09-24T22:00:00Z Brush night
PROBE[plan] 2026-09-24T22:10:00Z Sleep
```

Brush night lasts 300 seconds, ending at 22:05. Sleep starts at 22:10, 300 seconds
later, within its [0..1800] bound. The test checks actual plan timestamps and
step titles, not just the declarations or a response status.

Tests 3–6 cover every prescribed failure while retaining each requiring
routine, surviving explicit edges, both directions of tightest-bound merging,
absent maxima (omitted, never null), all four valid target collections and
invalid grammar variants, deterministic target sorting, a skipped establisher,
and self-establishment. Additional subcases check negative bounds and an empty
merged interval without discarding the routine.

## Quick UbU round trip

`routine_import_then_snapshot_preserves_declared_requirements` writes a
synthetic `routine.json`, runs `routine-import`, then `snapshot`, and asserts
the stored fields exactly:

```json
{
  "establishes": ["facts.teeth_clean"],
  "requires": [
    {"fact":"facts.fed","offset":null,"maximum":[14400,0]},
    {"fact":"facts.ready","offset":[60,0],"maximum":[600,0]}
  ]
}
```

The first requirement omitted `offset` in its input. Serde's default Option
round-trips it as null; the orchestrator accepts missing or null as None and
emits a zero minimum. A mainline absent maximum is omitted, not null.

## Judgment calls and literal readings

No disagreement with the seven intended choices: import-time ordinary edges,
nearest preceding nominal end, no arbitrary tie breaking, parallel declarations,
tightest merged bounds, reporting each success, and locally validating the
specified target grammar. The following ambiguities are explicit:

- Judgment 3 says “Report both” tied candidates, but C prescribes exactly
  `requirement_ambiguous: <target>` and D provides fields only for successful
  resolutions. We used that exact failure reason and the prescribed response
  shape. Candidate identities are not included; no id breaks the tie.
- The documented schema pattern is stricter than core's private parser in some
  character cases. The requested documented pattern is copied locally, with a
  synchronization comment, without adding a dependency or changing core.
- “No routine establishes it” means no eligible *other*, successfully mapped
  routine with parseable start and whole-second duration. This follows C's
  candidate rules and F's explicit skipped-establisher/self-reference cases.
  A later eligible establisher instead yields `unestablished_before`.
- Failure precedence is target grammar, fractional bounds, inverted bounds,
  then candidate resolution. Mixed-error requirements receive the first reason.
- The table omits negative minima and conflicts that arise only on merging.
  Core rejects negative minima. We classify these as inverted bounds and reject
  the offending requirement edge before admission; a conflicting intersection
  retains earlier valid edges. This follows the explicit edge-only failure rule
  rather than letting normalization discard an entire routine. Compatible
  intersections always use the largest minimum and smallest maximum.
- Each accepted requirement yields a resolution entry even if several merge to
  the same predecessor. The array is sorted by `(quick_ubu_id, target)`; mainline
  stores no establishes/requires fields or selection history. Existing dry-run,
  diverged and stale reporting remains unchanged.
- The ticket counts twelve `RoutineTemplate {` occurrences, not twelve full
  literals: those include the struct declaration, a helper's return type and a
  struct-update expression. All nine full literals gained the new empty fields;
  the struct-update expression inherits declarations via `..first.clone()`.
- The ticket calls its measured prototype “three meals, one brush, one sleep”
  while also specifying a two-brush table. We follow F exactly: three meals and
  two brushes in test 1, adding Sleep in test 2. The plan above is this run's
  actual synthetic output, not the ticket's sample timestamps. The supplied
  pinned counterexample is reproduced in `DECLARED_REQUIREMENTS.md` as supplied
  evidence, without pretending to rerun that prototype.

## Known limits (verbatim)

1. **This does not check preconditions.** A routine that establishes a target and one that destroys it are both invisible to the planner. `requires` orders work; it does not verify that the target is still true when the Task runs. The bounded chain from P1B-24 is what keeps an interloper out of the gap, and it does so by leaving no room, not by reasoning about state.
2. **Resolution is by nominal time, not realized time.** The establisher is chosen from declared `start_time` and `duration` at import. If a day actually runs so late that a different establisher would have been nearer, the edge does not move.
3. **Same-day only.** Nominal ends are compared as local wall-clock seconds within one day, so a routine just after midnight cannot resolve to an establisher from the previous evening. The existing `routine_after_unmatched` diagnostic still covers a predecessor whose recurrence does not land on the same date.
4. **The declarations live in Quick UbU.** `RoutineInstanceTemplate` has no `effects` field and occurrence Tasks carry none, so mainline has nowhere to record what a routine establishes. Until a mainline authoring surface exists, `routine.json` is the only place these can be written. The separable follow-up is to carry `establishes` onto the template and into the occurrence payload so that completing a routine actually mutates `UniverseState` through the existing `apply_completed_effects` path — that one is a `ubu-core` change and brings the seven-repo chain with it.
