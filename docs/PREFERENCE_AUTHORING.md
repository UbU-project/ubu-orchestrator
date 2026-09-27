# Preference authoring

A Preference is a durable pairwise statement about two Tasks: A is preferred to B, or A is indifferent to B. [UBU-D0282](https://github.com/UbU-project/ubu-design/blob/f7c4a1dbe2979fa498ae115bb2ee74e6772d4c2c/DECISIONS.md#ubu-d0282-phase-1b-task-prioritization-is-stored-as-pairwise-preferences) makes admitted, enabled Preferences the input to request-local priority layering. The resulting `TaskSpec.value` is transient: it belongs to the planning request, not the Task or Preference payload.

Native authoring removes the need to import every ranking from Quick UbU. New statements carry `acquired_method: user_defined`, an RFC 3339 `acquired_date` from the planning clock, and user provenance with no external `source`. The client cannot supply attribution, IDs, acquisition dates, or initial enablement. The importer therefore cannot mistake these statements for its own source-scoped records.

## Why cycles are rejected

The P1B-36 probe of the existing layering at `a8d0418` demonstrates a consistent chain:

```text
PROBE[chain] 01 value=1.0000
PROBE[chain] 02 value=0.5500
PROBE[chain] 03 value=0.1000
```

Closing the cycle erases the intended ranking and makes all three maximally valuable:

```text
PROBE[cycle] 01 value=1.0000
PROBE[cycle] 02 value=1.0000
PROBE[cycle] 03 value=1.0000
PROBE[cycle] CYCLE ["01", "02", "03"]
```

[UBU-D0289](https://github.com/UbU-project/ubu-design/blob/f7c4a1dbe2979fa498ae115bb2ee74e6772d4c2c/DECISIONS.md#ubu-d0289-phase-1b-partial-placement-keeps-valid-plans-when-optional-dynamic-work-does-not-fit) omits optional Dynamic work in lowest-value order. An erased ranking leaves the omission decision to deadlines and Task IDs, an arbitrary drop relative to the user's intended ranking. Rejecting the cycle when it is authored gives the operator a chance to resolve it immediately. `preference_cycle` remains an unchanged plan-time backstop for imported data.

## API and rejection order

All JSON writes require `schema_version: "ubu.orchestrator.preference.v1"`.

- `POST /preference` accepts `task_a`, `task_b`, and `order` (`a_preferred_to_b` or `a_indifferent_to_b`). It returns HTTP 201 with `schema_version`, `preference_id`, and `version`.
- `GET /preferences` returns a `preferences` array containing enabled and disabled statements, IDs, current Task titles, order, enabled state, acquisition timestamp, and version. It is a plain read with no paging.
- `PATCH /preference/:preference_id` accepts `schema_version`, required `expected_version`, and `enabled`. A stale version returns HTTP 409 with `version_conflict`; unknown Preferences return 404. Re-enabling runs the same subject and graph checks as creation.
- `DELETE /preference/:preference_id` takes no body and returns HTTP 204 with an empty body. An unknown or non-Preference ID returns 404.

Semantic validation follows this order, before canonical admission:

| Code | Reason and recovery |
| --- | --- |
| `preference_self_reference` | The subjects are identical. Supply two different subjects. |
| `preference_unknown_task` | A named Task is unknown or inactive, or a Task subject is missing. The diagnostic names a supplied invalid ID. |
| `preference_objective_pair_unsupported` | Planning does not yet consume Objective pairs. Use Task subjects. |
| `preference_duplicate_pair` | An enabled statement already expresses this pair/order. The diagnostic names its Preference ID. |
| `preference_contradiction` | An enabled statement expresses the reverse strict relation or conflicts with indifference on the same pair. The diagnostic names its Preference ID. |
| `preference_cycle_rejected` | The enabled graph is inconsistent. The diagnostic shows a closed cycle in relationship order. |

Duplicate/contradiction checks examine enabled statements only. Reversed indifference pairs are duplicates, since indifference is symmetric. A strict relation versus indifference is a contradiction, whichever is proposed first. Larger cycles reuse the existing layering detector, including its indifference merging; a small helper reconstructs an ordered witness from the already detected component. A branched inconsistent component can contain several cycles, so the diagnostic reports one actual closed cycle rather than implying every component member forms a single path.

The graph includes all Task subjects of active, enabled Preferences, not just today's eligible Tasks. An already imported enabled cycle can therefore block a new enabled statement until it is disabled or withdrawn. Disabling and deletion remain available to resolve it. Authoring is serialized with Quick UbU imports and Task lifecycle actions.

## Disable, withdraw, and visibility

Disabling preserves the statement while temporarily removing its planning effect. Deletion withdraws the Preference row itself. A Task records work and its lifecycle, so P1B-27 does not delete Tasks; a Preference records a ranking the operator may simply cease to hold. This difference is deliberate. Device mutation receipts remain as infrastructure, not as a retained Preference or tombstone. The store has no public deletion writer; the orchestrator's deletion statement is strictly restricted to Preference rows, without changing another repository or dependency.

Objective pairs are rejected even though the canonical schema can represent them. Supporting them requires a defined Objective-to-Task value policy and a planner implementation that consumes it, not merely an API that stores the pair. Previously imported Objective pairs remain readable in the list using `objective_a`/`objective_b`; Task fields are null for those entries. A missing Task title is null rather than hiding the statement.

An enabled Preference that later names a Task absent from the eligible planning set emits `preference_ignored_unknown_task`, naming the Preference and missing Task. This includes completed or rejected Tasks and other current eligibility exclusions. If both subjects are missing, each receives a diagnostic. The statement itself remains untouched. Disabled statements do not produce this warning. Clients supplying an explicit planner request retain their existing bypass of store-derived layering.

## Known limits

1. **Pairwise only.** There is no way to say "these five are all more important than those three" except as pairs, and the number of pairs grows quickly.
2. **No strength.** `a_preferred_to_b` says which, never by how much. The value gradient comes from the layering, not from the operator.
3. **`a_indifferent_to_b` is accepted and stored but carries no special handling here** beyond what `layer_preferences` already does with it.
4. **Objective Preferences are unreachable.** The schema allows them; planning does not read them; this endpoint rejects them.
5. **Cycle rejection is write-time only for this endpoint.** A cycle arriving through `/import/quick-ubu` is still only reported at plan time.
6. **No bulk authoring.** Ranking a fresh backlog means one call per pair.
7. **Titles in the listing are a snapshot.** Renaming a Task changes what the next listing shows; nothing is cached, but nothing is versioned either.

