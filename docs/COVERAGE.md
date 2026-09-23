# Coverage

`display_probability` measures how often a sampled day runs exactly as written.
Coverage also credits days where dropping optional work leaves a valid way to
continue. A Plan can therefore have much higher coverage than feasibility, without
changing its original rollout scores or pretending that every Task will finish.

## Two clocks and two continuations

Each rollout draws a duration once per Task, using the existing shared latent
correlation samples. The original clock carries lateness forward and applies the
existing anchor, Task-window and request-window checks. A second clock checks the
same windows but can omit optional work that no longer fits. Dropping work does
not advance that clock. Its dependents cannot continue without it; a mandatory
or Static Task that cannot continue makes the continuation fail.

This certifies the Plan as written and the Plan with displaced optional work
omitted. Coverage is the share of rollouts with either continuation. It is never
less than `display_probability` for a generated, validated candidate. No separate
sampling pass, randomness, invented duration spread or new compute allocation is
introduced.

## Boundary outcomes

The candidate's own Static placements define boundaries. The summary includes
those whose starts are at or before planning start plus the reactive horizon,
clamped to the planning window. A boundary state carries its index, start and
Task ref, the sorted set of work completed by that time, and lateness rounded up
to whole minutes. On-time arrival has a distinct zero bucket. The wire records
`lateness_seconds_ceil_60`, and deterministic digests fold explicitly encoded
fields rather than memory layouts.

A merged state counts as covered only if every sampled visit to it continued.
A mixed state counts as uncovered; its failed visits divided by all rollouts
contribute uncovered mass. The overall estimate still measures successful
continuation rollouts, with a 95% Wilson interval over those same samples.
Consequently state counts are not a substitute for the probability estimate.
Each listed boundary receives attribution from the rollout's full continuation
result; boundary selection does not truncate the two walks. Boundary masses
must not be added across boundaries, since they may describe the same samples.

## API and risk report

`selected_candidate.coverage` (also retained on stored Plan candidates and
alternatives) carries scope, estimate, uncovered mass, threshold, below-threshold
flag, confidence bounds, sample count, quantization rule, outcome counts, and
boundaries. Each boundary has the Task id and title, start seconds and RFC 3339
`start_at`, outcome counts and uncovered mass. A missing title falls back to the
id. Candidates without rollouts have null coverage.

Low coverage adds one non-blocking `low_coverage` finding. Severity is High below
half the target and Medium otherwise. The finding names the boundary with most
uncovered mass; ties prefer the later start then larger id. The id is carried in
`subject_ref`, and the detail uses the title:

> this Plan holds for 53% of the ways the next 60 minutes could go, below the 99% it aims for; most of the rest stops at Standup

Horizons up to 90 minutes are displayed in minutes; longer horizons use hours.
Without a sampled boundary, the detail omits the commitment clause rather than
inventing a commitment. Low coverage never withholds the Plan or raises a
recalculation during generation. The execution-time low-coverage trigger remains
available for later observed decay; regenerating unchanged fragility would loop.

## Policy

`horizon_policy` defaults to `reactive_horizon_seconds: 3600` and
`branch_coverage_target: 0.99`, from the kernel's exported constants. Store-built
requests use those defaults unchanged. Supplied requests may override either
field; omitted fields retain their defaults. Zero horizon and targets outside
`(0, 1]` are rejected. No Setting is introduced. Repair forwards the same policy.

## What this slice does not yet do

This is the accounting slice of UBU-D0285. It does not search alternative
continuations for uncovered states, rank them by probability, allocate competing
compute budgets, or package mobile continuation refs. `budget_limited` is false
because accounting uses completed Stage 4 samples. It does not model external
interruptions, splittable progress, or execution-conditioned coverage decay.

Quick UbU point-duration imports remain fixed durations. A fixed-duration Plan
produces identical rollouts and both numbers read 1.0 when it fits. Meaningful
uncertainty on those Tasks requires a separate execution-Log duration-modeling
ticket; importing an invented spread would fabricate the user's estimates.
