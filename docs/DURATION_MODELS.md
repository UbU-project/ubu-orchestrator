# Planning routine durations from observed execution

A fixed duration gives every Monte Carlo rollout the same work duration. When
that day fits, `display_probability` and coverage are 1.0: those confidence
numbers cannot express uncertainty that was never supplied. Quick UbU imports
point durations, so this used to apply even to routines that regularly ran late.

A routine now supplies its own evidence. Pressing `start` records that work
began, with user authority, without changing lifecycle state or applying Task
effects. A later `complete` closes an observation. Completion without an open
start is ignored; a new start replaces an unclosed start; an unclosed start at
the end is not an observation. Starts sort before completions at the same time,
so a same-instant interval measures nothing and is discarded. Starts are never
inferred from the Plan: that would manufacture punctual starts precisely when
a late day makes truthful observations most valuable.

Recorded and legacy actions use the planning clock. Production uses the system
clock; tests and replays can supply `FixedClock`. The reader accepts legacy
`task_started`/`task_done` events and `decision_recorded` actions
`start`/`complete`, joining each Log's first object ref to its Task's routine
Objective. Malformed timestamps are skipped. One-off Tasks do not belong to a
routine group and never receive a model.

Observations are grouped by **routine Objective id**, the entity that recurs,
not by category, tag, or title. Events from that group are paired chronologically.
Only Dynamic occurrence requests can receive the derived duration; Static
windows remain fixed. Constants are deliberate conservative thresholds:

| Constant | Value | Reason |
| --- | --- | --- |
| `MINIMUM_OBSERVATIONS` | 5 usable completed observations | Below this, a spread is too little evidence |
| `OBSERVATION_WINDOW_SECONDS` | 90 days | Older observations no longer describe current execution |
| `OUTLIER_MULTIPLE` | 8 times the pre-filter median | Remove forgotten timers while retaining genuine long days |

Both event timestamps must be in the inclusive window ending at planning now;
future events are excluded. The estimator rejects zero and reversed intervals,
checks the minimum, sorts durations and calculates a median over all usable
samples. It removes durations strictly above eight times that median, then
checks the minimum again. The diagnostic's observation count is the surviving
count. An untrusted group leaves the declared estimate unchanged.

The minimum, median, and nearest-rank 95th percentile become the three estimate
points. Nearest-rank selects element `ceil(n * fraction)` (one-based), including
the lower middle sample for an even-sized median. These are order statistics,
not fitted parameters pretending to precision that a handful of samples cannot
support. The existing shifted-lognormal model consumes those points unchanged.
Tied points receive only the one-second nudges required by its strict
`min < mode < p95` schema. If minimum and p95 are equal, the result stays `Fixed`;
no observed spread is invented.

An outlier ceiling is different from trimming the largest fraction of samples.
Trimming ten percent would discard genuine long days and **understate the tail**,
making a Plan falsely more certain. A forgotten timer is wrong by an order of
magnitude. The median-based ceiling excludes that case without routinely shaving
away the evidence about slow days.

Only transient planning request metadata changes. Both the declared Task and
its Objective's `routine_instance_template.duration_estimate` are retained.
This follows [UBU-D0282](../../ubu-design/DECISIONS.md#ubu-d0282-phase-1b-task-prioritization-is-stored-as-pairwise-preferences):
admitted inputs remain declarations while the CPU derives request metadata,
as it already does for Preference-based `value`.

The planner receives the observed mode as `duration` and the derived estimate
as `duration_estimate`. Each affected Objective gets one diagnostic, in stable
Objective-id order, even if several occurrences appear in the request. The
Objective title is used, falling back to its id:

```text
duration_model_observed
`Shower and dress` is planned from 6 observed runs, not its declared duration (usually 17 min, 30 min at worst, 15 min at best)
```

Display minutes round up; model seconds retain their precision. A constant
model says `N min every observed run`. “At worst” follows the ticket's wording
for p95; it is **not a hard maximum**. The existing distribution can sample beyond
p95, as specified in the kernel contract.

The ticket's motivating worked example, declared at ten minutes and observed
six times at fifteen to thirty, reports:

| Measure | Declared | Observed |
| --- | --- | --- |
| `display_probability` | 1.00 | 0.375 |
| coverage | 1.00 | 0.375 |
| uncovered mass at the commitment | — | 0.625 |

These are ticket-supplied example numbers. The HTTP test's measured values are
recorded separately in `P1B-23_VERIFICATION.md`; generated request ids seed the
rollouts, so individual runs can differ. The kernel itself is unchanged.

## Limits

- One-off Tasks never get a model. Their declared point durations can still
  make a fitting day report 1.0.
- Nothing is derived until at least five explicit starts have paired with
  usable completions in the recent window and survived filtering. A cold store
  deliberately remains unchanged while evidence accumulates.
- A start is a second press. If starts go unpressed, make starting cheaper on
  the next-action surface; do not infer starts.

This slice applies evidence immediately before Preference derivation, after
existing eligibility checks, as specified by P1B-23. It does not revive Tasks
already excluded using their declared duration or rerun those prechecks with
the observed estimate; the kernel validates the resulting request.
