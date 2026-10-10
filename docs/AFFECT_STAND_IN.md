# An affect state that was never recorded

A Plan is scored against an affect profile and an affect observation. The
observation comes from the latest Snapshot. When no Snapshot covers the
profile's dimensions, or the one there is has gone stale, the orchestrator
manufactures a stand-in so that the kernel has something to score:
`bootstrap_affect_observation` places a value on each tolerance's own
location and stamps it `source_kind: "bootstrap_default_profile"`.

## Why the stand-in reads as exactly zero

A tolerance's own location is the definition of "no information". In the
kernel, satisfaction is `sigmoid((value − location) / scale)`. At the location
that is `sigmoid(0)`, one half. Every tolerance this service builds has a
threshold of one half. So the margin is `0.5 − 0.5`, **exactly 0.000, in every
dimension, on every run**. A unit test asserts the arithmetic.

Until P1B-56 that zero was read as a measurement that had landed precisely on
the edge:

- `0.0 < 0.15`, so `post_plan_state_delta` was `depleted`;
- `0.0 <= 0.10`, so the `affect_margin` finding fired;
- `depleted`, so the `post_plan_depletion` finding fired;
- the stretch was not comfort, so "Add recovery time or reduce the plan's
  stretch load." was suggested.

Every Plan made with no Snapshot carried all four, beside a Plan that was
fine.

## What the reports do now

`derive_reports` asks whether the request's observation is a stand-in: every
dimension stamped `bootstrap_default_profile`. It reads the `source_kind`, not
the warning text. One really observed dimension is a measurement, and keeps
everything below live.

When the observation is a stand-in:

| | value | why |
|---|---|---|
| `affect_margin` | `0.0`, unchanged | the schema requires a number; this is the stand-in's |
| `post_plan_state_delta` | `neutral` | nothing was measured, so nothing is projected |
| `stretch_pressure` | `sustainable_stretch` | the enum has no "unknown"; `comfort` would be a claim about the operator |
| `affect_margin`, `destructive_pressure` and `post_plan_depletion` findings | not raised | all three read the manufactured number |
| `revision_suggestions` | begins with the sentence below; no recovery advice | a repair for a load nobody measured is not advice |

```text
Record how you are feeling: no affect Snapshot covers this Plan, so its affect margin, stretch pressure and post-plan state are a stand-in and not a measurement.
```

That sentence is first, whatever else is suggested.

`affect_margin` stays a required number because making it nullable is a
change to `ubu-schemas` and `ubu-core`. The UI reads the three figures as
"not recorded" when the legitimization warning says the stand-in was used.

## What is unchanged

- **`stale_affect`.** It fires on stale dimensions and on a warning containing
  "stale affect". A missing observation warns "missing affect observation;
  using bootstrap default profile observation in warn_only mode", which does
  not contain it. A stale Snapshot still raises `stale_affect`, and the affect
  figures beside it are the stand-in's, said to be so.
- **A real observation.** A Snapshot that happens to sit exactly on every
  location gives the same margin of zero, and every finding above is raised:
  it was measured.
- **A request supplied in full.** It carries its caller's own observation and
  is judged on it.
- **The legitimization report**, including its warning and its `warn_only`
  mode.

## How the stand-in retires

[The affect observation route](AFFECT_OBSERVATION.md) records an immutable
user-declared Snapshot. The next explicit Plan generation reads the latest
active Snapshot carrying affect; a newer Snapshot without affect cannot
hide it. A current complete live observation keeps the existing report
figures live and removes the stand-in sentence. Recording does not recalculate
an earlier Plan.

Uncalibrated priors use warn_only with or without a reading; any calibration
Setting present makes the profile enforce. Missing or stale observation
fallback stays warn_only. An observation is current until replaced when no
freshness limit is configured; a store that holds one, or a supplied request,
still goes stale. No freshness Setting writer or confidence decay is added.
The frozen Snapshot confidence-field gap is documented in UBU-D0304 and the
new route contract. The rehearsal's ordering remains a separate synthetic input.
