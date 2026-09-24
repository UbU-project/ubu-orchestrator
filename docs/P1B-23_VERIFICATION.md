# P1B-23 verification

Baseline: `ubu-orchestrator` at `8ec5d70b2f523a6822c507df7c518caee14fa6e2`.
Branch: `p1b-23-duration-models`. Only this repository changes.

## Checks and scope

- `cargo test --all-targets`: **186 passed**, from 176. Six pure estimator unit
  tests and exactly four new HTTP tests account for the increase.
- Every lettered section passed the full suite before its commit: A at 176;
  B, C, D and E at 182; F at 186.
- `cargo clippy --all-targets`: **10 before, 10 after**. Count distinct emitted
  warning diagnostics, excluding `generated N warnings` summaries and duplicate
  target passes. Separate source locations count separately even when messages
  match (including the two clone-on-Copy warnings). Before/after diagnostic
  message multisets are identical: no warning was added or fixed.
- The ticket's claimed baseline of 12 did not reproduce. P1B-22 recorded the
  same measured ten-warning result at this exact starting revision. Toolchain:
  rustc 1.98.1 / clippy 0.1.98 (`48a229cea`, 2026-09-01).
- `Cargo.toml` and `Cargo.lock` are byte-identical to the baseline; every git pin
  is unchanged and no dependency was added. Lockfile SHA-256:
  `486369199237d3d21ea39523f58204be7a6287a24a4d67264c76c24fa19d3bbe`.
- `cargo run --example generate_openapi` succeeded. The generated artifact
  changes only by adding `start` to `RecordedTaskActionKind`.
- Only the two new Rust files were formatted with `rustfmt --edition 2021`.
  Their format check and `git diff --check` pass. Existing formatting divergence
  and compiler concurrency settings remain untouched.
- All eleven read-only sibling repositories retain their initial revisions and
  clean trees. Every fixture is synthetic; no real Quick UbU file was read or
  committed.

## Execution evidence

The shared HTTP helper generates a Plan on each of September 1–6, finds the
active routine occurrence, records `start` at 09:00 UTC, then `complete` after
900, 960, 1020, 1200, 1500 and 1800 seconds respectively. Every call carries
`TASK_ACTION_SCHEMA_VERSION` and drives `FixedClock` to that action's time.
The tests read the resulting Log timestamps, verify user authority, no start
transition, unchanged Task payload after starting, and applied completion.

On September 7 the declared ten-minute routine is planned for **1020 seconds**,
inside the observed range and different from the declaration. The request
contains `{ min_seconds: 900, mode_seconds: 1020, p95_seconds: 1800 }` as a
`shifted_lognormal_p95` estimate. Test 1 prints this verbatim diagnostic:

```text
`Shower and dress` is planned from 6 observed runs, not its declared duration (usually 17 min, 30 min at worst, 15 min at best)
```

The Objective title deliberately differs from the occurrence title, proving
the name comes from the Objective lookup. The test also skips a malformed
legacy timestamp, re-encodes the synthetic history in `task_started`/`task_done`
form and obtains the same result, then plans two occurrences in one request
with exactly one Objective diagnostic.

Test 2 finds both the Objective template and seventh occurrence declaration
unchanged. The verbatim stored
`routine_instance_template.duration_estimate` value is:

```json
{"seconds":600,"type":"fixed"}
```

Test 3 verifies a history-free routine stays at exactly 600 seconds without a
diagnostic. It additionally exercises legacy `/start` and `/done` timestamps
under `FixedClock` and verifies a one-off Task retains its declaration.

Test 4 adds a Static commitment at 09:18–09:23 UTC; the routine's allowed range
is 09:00–09:18. Its captured passing run prints the following exact values:

```text
P1B23_BEFORE {"boundary.uncovered_mass":0.0,"coverage.estimate":1.0,"display_probability":1.0}
P1B23_AFTER {"boundary.uncovered_mass":0.749,"coverage.estimate":0.251,"display_probability":0.251}
```

`coverage.below_threshold` is true. The first boundary is the commitment
`task_018f3c8e9b2a7c4d8f1e2a3b4c5d8e02`, with non-zero uncovered mass. The
kernel and probability calculations are unchanged; only the routine's request
duration model differs. Generated request ids seed rollouts, so measured
fractions vary between runs. The ticket's 0.375/0.625 worked example is retained
as supplied in `DURATION_MODELS.md`, not treated as an exact test golden: the
ticket does not prescribe the six exact durations or commitment offset.

The six unit tests cover insufficient counts before and after filtering,
instantaneous/reversed intervals, nearest-rank order statistics and strict
ordering nudges, a forgotten timer versus a retained genuine long day,
constant work, all pairing rules, deterministic grouping, inclusive ageing
boundaries and future-event exclusion.

## Judgment calls

No disagreement with the nine judgment calls. Stored declarations remain
unchanged; starts are explicit; starting changes no lifecycle state; grouping
uses the routine Objective; estimates use order statistics; outliers use a
median multiple rather than trimming a fraction; five observations and ninety
days gate trust; constant evidence stays fixed; both action writers use the
planning clock.

## Literal readings and ambiguities

- The ticket calls the legacy writer `record_action`; its service name is
  `append_action`. Updated that actual writer and `record_task_action`.
- A usable observation is a strictly positive interval. Both event endpoints
  must be within `[now - 90 days, now]`, inclusive. A start before the cutoff
  cannot pair with a retained completion; future events do not count.
- The specified row shape discards occurrence identity after grouping by
  Objective. Pairing therefore runs across that Objective's event stream; it
  does not introduce an additional per-occurrence partition. Starts at equal
  timestamps precede completions, as explicitly required.
- The median uses nearest-rank 50%, including the lower middle sample for even
  counts. The outlier ceiling uses the median before filtering and removes only
  values strictly above eight times it. Counts in diagnostics are survivors.
- Tied minimum/median/p95 values receive the smallest upward whole-second
  nudges needed for strict schema ordering. A one-second-wide nonconstant spread
  may move p95 upward by one second. `min == p95` takes the explicitly required
  fixed branch without inventing spread, even if a sample above p95 exists.
- Whole display minutes round up. “At worst” is the ticket's exact diagnostic
  wording for p95, not a hard cap; the existing distribution can exceed p95.
  Constant models say `N min every observed run`.
- `by_group` and diagnostic emission are deterministic for the same Log and
  clock. Existing request ids and rollout seeds remain newly generated; this
  ticket does not make the entire pre-existing request envelope byte-identical.
- Applied models immediately before `active_task_preferences`, the specified
  insertion point. Earlier declaration-based eligibility checks are unchanged;
  excluded Tasks are not revived and those checks are not rerun. Final kernel
  validation still sees the derived estimate.
- A group emits a diagnostic only when at least one eligible Dynamic occurrence
  actually receives its model. Static windows and one-off Tasks keep their
  existing duration semantics. Objective title lookup falls back to the id.
- The Clippy absolute baseline discrepancy is recorded above rather than
  manufacturing warnings to reach twelve.

## Known limits

1. One-off Tasks never get a model. Their declared point durations remain, so
   a fitting day containing only that work can still report 1.0.
2. Nothing is derived until the fifth explicit start has a usable completion
   and five recent observations survive filtering. A cold store is deliberately
   inert while data accumulates.
3. A start is a second press. If starts go unpressed, the follow-up is to make
   starting cheaper on the next-action surface, not to infer starts.
