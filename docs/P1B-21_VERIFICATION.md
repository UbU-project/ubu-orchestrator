# P1B-21 verification

Both repositories use branch `p1b-21-coverage`. The kernel was landed and pushed
first at `1cae4f0c82013a48b7899ce3050fbd7bab094e75`. Both orchestrator dependencies pin
that exact revision. Sections A–E are kernel commits, F–H orchestrator commits;
this verification record belongs to section H.

## Checks and repository constraints

| Check | Result |
| --- | --- |
| Kernel `cargo test --workspace` | **81 passed**, zero failed; baseline 73 |
| Kernel `cargo clippy --workspace --all-targets -- -D warnings` | **0 warnings**, passed |
| Kernel `cargo fmt --check` | Passed |
| Orchestrator `cargo test` | **172 passed**, zero failed; baseline 169 |
| Orchestrator `cargo clippy --all-targets` | **11 distinct diagnostics before and after**, unchanged |
| `cargo run --example generate_openapi` | Passed; schema regenerated |
| Kernel Cargo.lock | Byte-identical to `eba24e377df7d663b9d2c25d086cd6cdd57d1a18` |
| Orchestrator Cargo.lock | Exactly the two kernel source revision lines changed |
| Other git pins and packages | Unchanged; no new dependencies |

The warning count excludes Cargo's `generated N warnings` summaries and duplicate
build passes, retaining separate diagnostics at separate source locations.
The measured baseline at `4126fd4` is **11**, not the ticket's stated **13**:
eight library diagnostics, one additional library-test diagnostic, and two in
`tests/routine_instantiation.rs`. Before/after diagnostic-message multisets match
exactly, including the two distinct clone-on-Copy warnings. No existing warning
was fixed or new warning added. Toolchain: rustc 1.98.1 / clippy 0.1.98
(`48a229cea`, 2026-09-01).

Kernel lockfile SHA-256:
`9d69bb3a64e9627134ef01fd8848e938302513b286f2ed8447f69dcb21eb2ccc`.
The orchestrator lockfile is exactly its baseline content after replacing only
the two kernel revision source lines; its manifest similarly changes only the
two pins. Kernel Cargo.toml is unchanged. No path dependencies, patch configs,
update-all runs, or compiler/test concurrency caps were introduced.

All 12 sibling working trees were clean before work. The ten read-only siblings
retain their exact initial revisions and clean trees. All inputs are synthetic;
no test reads a real Quick UbU file. Kernel fixtures and the six stale,
uncompiled repository-root test files remain byte-identical. Only the new
orchestrator Rust test was formatted, preserving existing formatting divergence.

Every section was committed after its full suite passed: kernel A–D at 73 and
E at 81; orchestrator F–G at 169 and H at 172. Struct-literal compatibility
updates moved into the section introducing each field to keep commits green.

```text
0d6a733 P1B-21 A: add validated reactive horizon policy
7a8c122 P1B-21 B: account for deterministic boundary outcome coverage
aca1558 P1B-21 C: measure continuation coverage beside unchanged feasibility replay
6ab40cb P1B-21 D: document outcome coverage and certified continuations
1cae4f0 P1B-21 E: certify boundary coverage and optional continuation gain
6dd8553 P1B-21 F: expose horizon policy and candidate coverage in the API
4ac75c0 P1B-21 G: report low continuation coverage without blocking or regeneration
```

## Verbatim HTTP evidence

`tests/coverage.rs::uncertainty_names_commitment_without_blocking_or_recalculation`
printed this exact `selected_candidate.coverage` object:

```json
{"below_threshold":true,"boundaries":[{"covered_outcome_count":1,"start":1790070000,"start_at":"2026-09-22T09:40:00Z","summary":"Standup","task_id":"task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e02","uncovered_mass":0.188,"uncovered_outcome_count":46}],"confidence_high":0.8350052901162315,"confidence_low":0.7866068125887822,"covered_outcome_count":1,"estimate":0.812,"n_rollouts":1000,"quantization_rule":"lateness_seconds_ceil_60","scope":"reactive_horizon","threshold_used":0.99,"uncovered_mass":0.18799999999999994,"uncovered_outcome_count":46}
```

Its exact `low_coverage` detail:

```text
this Plan holds for 81% of the ways the next 60 minutes could go, below the 99% it aims for; most of the rest stops at Standup
```

The test verifies a real 1,000-sample interval bracketing the estimate, one
boundary carrying Standup's title and RFC 3339 start, Medium non-blocking severity,
no blocking findings, and no recalculation log. A supplied-request variant checks
High severity below half the threshold without becoming blocking. The other
HTTP tests check comfortable full coverage, stored candidate persistence,
default policy forwarding and repair overrides, disabled rollouts, a second
commitment four hours out remaining outside the default scope, and a wider
five-hour policy selecting the later boundary when uncovered masses tie.
Store-generated request ids seed the rollouts, so repeated HTTP runs may produce
slightly different estimates; the quoted object is the captured passing run.

## Coverage is not renamed feasibility

The kernel's
`coverage::optional_omission_is_a_certified_continuation` printed:

```text
P1B21_GAP display_probability=0.228 coverage_estimate=0.825 gap=0.597
```

Coverage is **82.5%**, versus **22.8%** display probability: a **59.7 percentage-point**
gain from the optional-omission continuation. The slow work is mandatory and the
commitment fixed; the optional work after that commitment can be dropped when
its sampled duration misses its allowed window. The companion protected-only
test requires exact equality when there is nothing droppable. All prior goldens
pass unchanged: duration sampling, seed streams, feasibility and scores retain
their previous meaning.

Eight new kernel tests also cover the comfortable case, loss attribution and
Wilson interval, zero versus ceil-minute lateness, a fixed digest vector and
field sensitivity, insertion-order-independent counter summaries and digest
ties, conservative mixed-state counts, reactive-boundary selection, disabled
rollout coverage, policy defaults and invalid policy rejection.

## Ten judgment calls

No disagreement with the ten judgment calls; implemented as specified:

1. This is the outcome-accounting slice of UBU-D0285; broader continuation search remains future work.
2. Certify the as-written and optional-omission continuations; protected work cannot be dropped.
3. Derive boundaries from the candidate's Static PlanSteps inside core.
4. Record deterministic ceil-60-second lateness quantization on the wire.
5. Keep budget_limited false because all existing samples are accounted for.
6. Low coverage is non-blocking; High only below half the target.
7. Raise no generation-time recalculation for low coverage.
8. Default policy to 3,600 seconds and 0.99, with supplied overrides and no new Setting.
9. Use the existing 95% Wilson estimator and report its method and sample count.
10. Leave feasibility semantics and all fixture values unchanged.

## Known point-duration limit

As described in the ticket, real imported Quick UbU Tasks carry point durations:
`est_duration` is seconds plus a nanosecond component that is zero, and imports
as `{"type":"fixed"}`. A fitting Plan then has coverage **1.0** and
`display_probability` **1.0** because every rollout is identical: Stage 4 runs a
thousand copies of the same day. The fixed-duration HTTP test verifies that
mechanism using synthetic data; real private data was not read or committed.
The machinery is correct but inert on those inputs until Tasks have three-point
estimates. No spread was invented during import. Deriving duration models from
the execution Log belongs to a separate ticket.

## Literal readings and ambiguities

- The reactive limit selects reported boundaries, as section C specifies. Both
  clocks still walk the full candidate, and the final continuation result is
  associated with each visited boundary. This slice does not truncate continuation
  checks at the reactive limit or locate the first failing boundary. The same
  failing rollout may contribute to multiple boundaries; their masses must not
  be summed. With no boundaries the list is empty and the risk detail omits a
  commitment clause instead of inventing one.
- A state is covered only if every visit continued; a mixed state is uncovered.
  The overall estimate is successful continuation rollouts / all rollouts, and
  boundary uncovered mass is failing visits / all rollouts, per section B.
  State-count certification and sample probability are deliberately distinct.
- “Completed so far” means sampled work ending by the boundary's scheduled start,
  stored as a sorted set. Pending work is not falsely marked complete just because
  the simulation has computed its eventual end. Boundary indices count Static
  placements in deterministic time order.
- Digests fold index, start, length-prefixed Task ref, completed-set length and
  sorted length-prefixed refs, then lateness bucket. Integers are explicit u64
  little-endian bytes; string lengths are UTF-8 byte counts. Lateness buckets
  count whole minutes; on-time and negative differences map to zero.
- Continuation dependencies must be present in its own completed-end map. An
  optional dependent of dropped work is also dropped; a protected dependent
  fails continuation. This preserves P1B-20's dependency rule.
- Wilson uses method `wilson` and confidence_level 0.95. Before any observations,
  the public counter reports estimate zero and an uninformative [0,1] interval;
  normal rollout candidates are never summarized with zero samples.
- Kernel defaults are authoritative. API field defaults call those constants;
  horizon policy is validated on both planning and repair, including unchanged
  repair candidates. Response schema version remains 0.1.
- Minutes are rounded up for display at horizons of at most 90 minutes; longer
  horizons are expressed as hours. Policy seconds are not mutated. Percentages
  are rounded only in user-facing text, not in the stored estimate.
- The ticket's 13-warning baseline did not reproduce. Preserving the measured
  eleven diagnostics follows the no-added/no-fixed-warning requirement rather
  than introducing warnings to reach a stale count.
