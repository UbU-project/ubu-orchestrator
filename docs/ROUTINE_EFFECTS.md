# Routine effects and reactive verification

A routine can now change UniverseState when completed and be gated by that state
when a Plan is built. Both `effects` and `preconditions` live on
`RoutineInstanceTemplate` and are copied onto occurrence Tasks. Occurrence
payloads are rebuilt from their template on each `materialize`, so editing only
an occurrence is not a durable way to author either field.

## Quick UbU declarations

```json
{
  "establishes": ["facts.teeth_clean"],
  "requires": [{"fact":"facts.fed","maximum":[10800,0],"verify":true}]
}
```

Each valid `establishes` target lowers to a `set_fact` mutation whose payload is
`true`. This declaration is limited to valid `facts.` targets. Non-fact or
malformed targets produce `establishes_invalid_target: <target>` at import;
only that mutation is skipped, and the routine and other valid mutations remain.
The P1B-25 resolver still handles ordering independently; a declaration rejected
for effect generation is not silently reinterpreted as another mutation kind.

`verify` defaults to false, preserving P1B-25's ordering-only behavior. With
`verify: true`, the requirement also lowers to an `equals true` precondition on
its target. One verified requirement is a bare leaf; several use `all_of`.
Verification remains independent of edge resolution: an unresolved requirement
still contributes its precondition, while import reports its resolution failure.
A mainline or one-off Task could establish that fact later. Invalid preconditions
remain subject to the existing gate's invalid-task reporting.

Verification is opt-in because every occurrence is mandatory (`UBU-D0288`).
Automatically verifying all existing requirements would make mandatory routines
unschedulable until their facts were established, potentially breaking the day
on a cold store. Quick UbU carries `verify` through import and snapshots; its own
scheduler still ignores it.

Mainline templates can use the existing richer Task effect syntax, including a
`clear_fact` destroyer. Numeric mutations, set memberships, event markers and
non-true values are not inferred from Quick UbU's list of target names.

## First-use state

Before this change, every declared Task effect on a store with no persisted
UniverseState was discarded with this diagnostic (the ticket's supplied probe
at `9410713`):

```text
PROBE[complete] diagnostics=[{"code":"task_effect_universe_state_absent",
  "message":"no current UniverseState exists; completed Task effects were not applied"}]
```

GitHub `/bootstrap/seed`, which requires a selected repository, was the only
creation path. Completion now seeds an empty UniverseState on first use, using
the completion's effective time, ordinary provenance and the completing Task's
compartment label. The seed is admitted through the store, re-read, and then
updated through the existing effect applicator and version-checked persistence.
This applies to ordinary Tasks as well as routine occurrences. No-effect and
mode-rejected actions do not create a seed. The absent-state diagnostic's sole
production emitter was removed; an unexpected missing state after admission is
an internal error, not a successful action with discarded effects.

## Reactive versus predictive

The ticket's supplied cold-store loop illustrates the intended behavior:

```text
PROBE[day] planned=["Brush and floss"]
PROBE[day] blocked=[{...,"target":"facts.teeth_clean"}]
PROBE[occurrence] effects={"mutations":[{"operation":"set_fact","payload":true,"target":"facts.teeth_clean"}]}
PROBE[occurrence] sleep.preconditions={"expected":true,"predicate":"equals","target":"facts.teeth_clean"}
PROBE[before] facts=(no UniverseState)
PROBE[complete] diagnostics=[]
PROBE[after] facts={"teeth_clean":true}
PROBE[after] planned=["Sleep"]
PROBE[after] blocked=null
```

Each `generate` re-evaluates against the real current UniverseState. Sleep is
blocked before brushing completes, becomes eligible after completion, and is
visibly re-blocked on the next generation if Dinner clears the fact. The existing
`partition_tasks_by_preconditions` gate is unchanged. Blocked mandatory work is
reported in `blocked_tasks` with its precondition; it is not capacity-omitted
work in `unplaced_tasks`.

Predictive enforcement would simulate state across a Plan and prove that its
preconditions hold. The earlier cheapest-link prototype instead reported
`status="ok"` even though a later destroyer invalidated the consumer's condition:

```text
PROBE[pinned] status="ok"
PROBE[pinned] link task_…6e02 <- task_…6e01
PROBE replay 2026-09-24T09:00:00Z Brush and floss   precondition=—
PROBE replay 2026-09-24T18:00:00Z Dinner            precondition=—
PROBE replay 2026-09-24T22:33:20Z Sleep             precondition=VIOLATED
```

That result was optimistic and wrong. Reactive gating can be over-conservative:
it refuses to schedule a consumer whose condition would become true later today.
It makes the current block visible instead of claiming that a dependency proves
future state. It is **a prerequisite for the Phase 3 enforcement work, not a down
payment on it**. No simulation, threat resolution or planner/kernel change is
introduced here. A Plan is checked against state at generation time; this does
not promise that facts cannot change before execution or replace regeneration.
The probes above are supplied evidence; this ticket's actual synthetic outputs
are recorded separately in `P1B-26_VERIFICATION.md`.

## Known limits

1. **Reactive, not predictive.** A precondition is checked against the world as it is when the Plan is built, not as it will be when the Task runs. A consumer whose establisher is planned for later the same day is blocked now and becomes eligible only after that establisher is actually completed. This is conservative by construction and is the opposite failure mode from plan-state simulation.
2. **`establishes` can only set facts true.** Numeric, set-membership and event-marker effects, and facts with values other than `true`, need a richer declaration than a list of target names.
3. **`success_probability` is still read nowhere.** A probabilistic effect applies in full on completion, exactly as it did for one-off Tasks. The field remains declared and unimplemented.
4. **A blocked mandatory occurrence is not `unplaced_tasks`.** It is reported in `blocked_tasks` with its precondition, which is the right diagnostic, but `UBU-D0289`'s partial-placement machinery never sees it and a caller reading only `unplaced_tasks` will not know the day is short one routine.
5. **The seed is empty.** A store that never bootstrapped gets a `UniverseState` with no facts, so every precondition over a target nothing has established yet is false rather than unknown. There is no distinction between "false" and "never observed."

