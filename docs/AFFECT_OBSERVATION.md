# A user-declared affect observation

A check-in records how the operator feels now. The governing contract is
PLANNING_KERNEL_CONTRACT.md §6: “It must not silently present stale affect
assumptions as current measured state.” UBU-D0304 records the mode and scale
rules. “Live observation” identifies a user-declared observed assertion; it
is not a claim of an instrument measurement.

## One path, two methods

`POST /affect/observation` accepts:

```json
{"schema_version":"ubu.orchestrator.affect_observation.v1","energy":7,"stress":3,"mood_intensity":3}
```

All three values are required, finite numbers from 0 to 10. Fractions are
allowed by the API; Today offers whole steps. Higher energy is better;
lower stress is better; mood intensity means arousal or volatility, not
valence. Positive excitement and negative agitation can both consume capacity.

HTTP 201 returns `schema_version`, `snapshot_id`, `observed_at`,
`source_kind: "live_observation"` and `dimension_count: 3`. The last two are
server metadata for the rehearsal's public projection; no value is echoed.

Missing/unknown schema versions use existing `missing_schema_version` and
`unknown_schema_version`. The two new HTTP 400 diagnostic codes are exactly
`affect_dimension_missing` and `affect_value_out_of_range`. Diagnostics name
the dimension and rule, never the supplied value. Unknown fields, including
client id, timestamp or source attribution, are refused by the request DTO.
There is no PATCH or DELETE.

`GET /affect/observation` returns HTTP 200:

```json
{"schema_version":"ubu.orchestrator.affect_observation.v1","observation":null}
```

or an observation with `snapshot_id`, `observed_at`, `source_kind` and
`dimensions: {energy, stress, mood_intensity}`, preserving the private values.

## Immutable admission and shared selection

The writer mints a Snapshot id and uses the ordinary User mutation envelope
and `queries::admit_object`: object_type Snapshot, version 1, status active,
compartment_label user-capture. Its typed core Snapshot has captured_at and
affect.observed_at equal to the planning clock's now, objects [],
affect.source_kind live_observation, and each dimension {dimension, value}.
Attribution is server-controlled. Every check-in creates a new row; none
edits the previous observation.

GET and store-built planning share the query for active Snapshots carrying
`affect` (`json_extract(payload_json, '$.affect') IS NOT NULL`), ordered by
updated_at descending, then local admission rowid descending for equal
clock times. A newer non-affect Snapshot cannot hide the latest reading.
The planner reads it on the next `POST /planning/generate`; nothing
recalculates on its own. Earlier Plans retain their original reports.

UBU-D0100 confidence gap: the frozen core/schema have no snapshot-level
confidence field. This implements immutable observed assertions, without
claiming full compliance with that decision's confidence policy.

## Calibration, mode and freshness

Uncalibrated store-built profiles use `warn_only`, even with live readings.
The warning remains “affect profile uses bootstrap default review priors;
review calibration recommended”. Energy 2 under default tolerances returns
a warned Plan, affect_feasible false, with energy violated, when the other
planning constraints permit a Plan. Calibrated enforcement and unrelated
failures may still return no Plan.

Calibration means any of the exact Settings checked by build_affect_profile
is present: acceptable_energy_floor, affect_energy_floor, energy_floor,
tolerable_stress_ceiling, affect_stress_ceiling, stress_ceiling,
tolerable_intensity_ceiling, tolerable_mood_intensity_ceiling,
affect_mood_intensity_ceiling, mood_intensity_ceiling. Explicit default
numbers, null and unparsable values still count as presence. Existing
numeric parsing and default fallback locations remain 4/7/8; calibrated
profiles enforce. Missing/incomplete/stale observation fallback remains
warn_only, with its stand-in provenance and warning. Supplied full kernel
requests retain their caller's mode and freshness limits.

An observation is current until replaced when no freshness limit is configured;
a store that holds one, or a supplied request, still goes stale. The current
Settings authoring gate and bootstrap have no freshness_seconds writer; none
is added here. Existing limits remain effective. UBU-D0039 confidence decay
remains unimplemented and is distinct from the implemented freshness check.

With a current live reading, unchanged planning_analysis derives live affect
margin, stretch pressure, post-plan state and affect findings. It emits no
“Record how you are feeling:” stand-in sentence. The rehearsal still labels
P1B-81's seeded ranking synthetic_stand_in; this route supplies the affect input.
