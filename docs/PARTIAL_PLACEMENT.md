# Partial placement

`/planning/generate` returns `status: ok` when a Plan has no unplaced work,
`partial` when a Plan survives with a nonempty `unplaced_tasks` list, and
`rejected` when there is no Plan. A partial Plan is a real admitted Plan: it is
legitimized, persisted, projected, recalculated and used by next-action normally.
The response status is separate from the stored Plan's `admitted` lifecycle.
The kernel reserves `engine_error` for backend failures outside CPU planning.

| Reason | Meaning |
| --- | --- |
| `insufficient_total_capacity` | Protected work holds the available time. |
| `no_eligible_chunk_large_enough` | No free interval can hold the whole atomic Task. |
| `outside_allowed_window` | The allowed range cannot hold the Task after prerequisites. |
| `omitted_lower_value` | Other optional work holds the time. |
| `deferred_dependency` | An omitted prerequisite prevents placement. |

The unified list starts with the kernel's report in its deterministic order,
then adds pre-dispatch exclusions not already represented. `task_unplaceable`
and `dependency_outside_horizon` map to `outside_allowed_window`;
`prerequisite_unplaceable` maps to `deferred_dependency`. Each original diagnostic
is retained, and its message becomes the explanation. Older pre-dispatch
messages do not identify individual prerequisite ids, so their reference lists
remain empty. `summary` is the stored Task title, with its id as fallback.
Safe alternatives come from the kernel and require user input.

Every entry creates a Medium, non-blocking `unplaced_work` risk finding.
Next-action recommends only placed Tasks; the stored report keeps unplaced
work visible alongside the recommendation. Structural failures remain blocking.

Mandatory routine occurrences, Static Tasks, fixed placements and their
transitive prerequisites are never selected for omission by the kernel.
`TaskSpec.mandatory` conveys this even when an occurrence has value zero.
Before dispatch, the orchestrator seats Dynamic mandatory occurrences together
against fixed commitments and previously seated occurrences, ordered by latest
finish then id with prerequisites settled first. Checking each occurrence alone
would miss conflicts between two routines needing the same time.

An occurrence that cannot be seated stands down through the existing
`mandatory_occurrence_unplaceable` diagnostic and non-blocking `routine_triage`
finding. It is absent from `unplaced_tasks`: it is required work needing a
routine decision, not optional work selected for omission. Dependents are
excluded by the existing fixpoint. Cycles remain subject to graph validation.

Optional omissions prefer lowest value, latest own deadline (absent deadlines
first), then largest id. Omission sets outrank optimistic utility. All returned
candidates omit the same set; greedy replaces the sweep when it gives up less,
joins for the same set, and is discarded if it gives up more. Dependents of an
omitted Task are deferred rather than scheduled independently.

The kernel never extends the horizon: no policy bound exists there and the
orchestrator owns the requested period. Capacity/window reasons record
`skipped_by_policy`; ranking/dependency reasons use `not_applicable`.
`unsupported_split_policy` and `horizon_extension_limit` remain future reasons.
The contract stays at `planning-kernel-contract/0.1` until split policy UBU-D0284
lands the shared 0.2 revision.
