# The UniverseState route

A Task can ask that something be true before UbU will plan it. That something
is a `UniverseState`: four collections of facts about the operator's world. The
planner evaluates a Task's preconditions against it, and a completed Task's
effects change it. Until P1B-58 nothing let the operator see it or change it by
hand. `GET /universe-state` and
`PATCH /universe-state` do.

**This document is contract.** A change to the route's operations, to the
predicates the planner evaluates, or to the provenance it records, changes this
file in the same commit.

## What "current" means

There is one definition, `planning_service::read_current_universe_state`: the
`UniverseState` row with the latest `updated_at`. The precondition path, the
effects path and this route all read through it. The route adds no second one.

That reader returns nothing when the store holds no `UniverseState`. The
planner then evaluates against an empty state it builds on the spot and never
stores (`planning_service::synthesized_universe_state`). The read route returns
that same empty state with `version: null`. Its `id` and `captured_at` are made
at each read, so they change from one read to the next and mean nothing until a
state is stored. A read never creates a row.

## Read

`GET /universe-state` returns:

| Field | Meaning |
|---|---|
| `schema_version` | `ubu.orchestrator.universe_state.v1` |
| `id` | The state's id. |
| `version` | The stored version, or `null` when nothing is stored. |
| `captured_at` | When the state was first captured. An edit does not move it. |
| `facts` | Key to any JSON value. |
| `numeric_values` | Key to number. |
| `set_memberships` | Key to a list of JSON scalars, in the set's own order. |
| `event_markers` | Key to a list of JSON objects, oldest first. |
| `fact_provenance` | Full target to `{kind, recorded_at}`: how the value there was established, and when. |
| `source_summary` | One sentence for the whole state. An edit does not rewrite it. |
| `confidence_summary` | One sentence for the whole state, or `null`. |

All four collections and `fact_provenance` are always present, empty or not. A
key is the part of a target after its collection: the fact a precondition
names as `facts.kettle.descaled` is stored under `kettle.descaled` in `facts`.
`fact_provenance` is keyed by the whole target, `facts.kettle.descaled`.

## Edit

`PATCH /universe-state` takes

```json
{
  "schema_version": "ubu.orchestrator.universe_state.v1",
  "mutations": [
    {"operation": "set_fact", "target": "facts.kettle.descaled", "payload": true}
  ]
}
```

and answers with the state after the mutations, in the shape of the read.

A mutation is `ubu-core`'s `UniverseMutation`, field for field: `operation`,
`target`, and optional `payload` and `provenance_kind`. The route defines no
operation of its own. The nine are the ones a Task's `effects` use:

| Operation | Collection | Payload |
|---|---|---|
| `set_fact` | `facts` | any JSON value, required |
| `clear_fact` | `facts` | none allowed |
| `set_numeric` | `numeric_values` | a number |
| `clear_numeric` | `numeric_values` | none allowed |
| `increment_numeric` | `numeric_values` | a number |
| `decrement_numeric` | `numeric_values` | a number |
| `add_membership` | `set_memberships` | a JSON scalar |
| `remove_membership` | `set_memberships` | a JSON scalar |
| `append_event_marker` | `event_markers` | a JSON object |

**A number is set and cleared outright**, from P1B-59. `set_numeric` replaces
whatever is there and `clear_numeric` removes the key. A reading from a gauge
is set, not reached from whatever happened to be there. Until then a number
could only be moved by a difference: from 0.7, a request for 0.1 arrived as
0.09999999999999998, and a number could never be removed. Clearing a key that
is not there changes nothing and is not an error. Increment and decrement
stay, for a tally, and a key that is not there counts from zero. There is no
operation that removes an event marker.

**A mutation has no `note`.** It was accepted and stored nowhere. A mutation
that carries one is refused, here with 422 and on a Task's `effects` with 400.

The order of work is fixed, and nothing is written until every check passes:

1. `validate_mutations_for_mode` for this instance's mode. An intrinsic-affect
   target, one whose second segment is `affect`, is refused outside
   `user_mode`. The MVP instance is `user_mode`, so the operator may set one.
2. `apply_universe_mutations` on the current state, or on an empty one when the
   store holds none. It validates the whole list before applying any of it.
3. Only then the write.

So a list with one bad mutation is refused whole. No earlier mutation in it is
applied, and on a store with no state no empty row is left behind.

| Status | Code | When |
|---|---|---|
| 400 | `missing_schema_version`, `unknown_schema_version` | The request names no version or another one. |
| 400 | `universe_mutations_empty` | `mutations` is an empty list. |
| 400 | `universe_mutation_mode_invalid` | Step 1 refused. The message is `ubu-core`'s. |
| 400 | `universe_mutation_invalid` | Step 2 refused. The message is `ubu-core`'s, for example `mutation 1: unknown operation ...`, counting from zero. |
| 409 | none | The state changed between the read and the write. |
| 422 | none | The body is not this shape, for example a mutation with a key the type does not have. |

## Provenance

Each value can carry how it was established:

| `provenance_kind` | Means |
|---|---|
| `asserted` | A person said so. |
| `measured` | An instrument or a reading. |
| `derived` | Computed from other facts. |
| `proposed` | An advisor suggested it and it has not been confirmed. |

There is no confidence number and no free text. The point of the field is to
tell evidence from assertion, and a score would blur that.

- **A mutation states the kind of what it writes, and none means `asserted`.**
  A mutation with no stated evidence is someone's word. Every request that
  worked before P1B-59 still works and means what it meant.
- **The write records the kind and the time** under the mutation's target in
  `fact_provenance`. The time is this orchestrator's clock at the write. It is
  not `captured_at`, which does not move.
- **A later write replaces the entry.** A measured number set again with no
  kind is asserted.
- **No entry outlives its value.** `clear_fact`, `clear_numeric`, and a
  `remove_membership` that takes a set's last member, remove the entry with
  the value. The two clears refuse a `provenance_kind`, as they refuse a
  payload: they write nothing for it to describe.
- **A set has one entry**, for the set and not for each member. A
  `remove_membership` that leaves members is a write to the set.
- **A completed Task's effects record it too**, through the same applicator,
  with the completion's time.
- **A value written before P1B-59, and each fact bootstrap writes, has no
  entry.** The map says nothing about it.

The map is `fact_provenance` and not `provenance`. Every stored object's
payload carries its envelope under `provenance`, a `UniverseState`'s included,
and the store rewrites that key on each write.

## Preconditions the planner evaluates

A Task's `preconditions` are evaluated against the current state when a Plan
is generated, by `ubu-core`'s `evaluate_universe_precondition`. This
orchestrator adds nothing to it and has no predicate of its own. A leaf has a
`target`, a `predicate` and, for all but `absent`, an `expected`:

| Predicate | Target | True when |
|---|---|---|
| `equals` | any | the value equals `expected` |
| `member_of` | `set_memberships` | the set holds `expected` |
| `absent` | any | nothing is recorded there |
| `at_least` | `numeric_values` | the number is `expected` or more |
| `at_most` | `numeric_values` | the number is `expected` or less |
| `greater_than` | `numeric_values` | the number is more than `expected` |
| `less_than` | `numeric_values` | the number is less than `expected` |

The four comparisons are from P1B-59. **A number that was never recorded
satisfies none of them**: the Task is in `blocked_tasks`, not ready, and it is
not an error to ask. A comparison on a target outside `numeric_values`, or
with an `expected` that is not a number, is malformed, and the Task is in
`invalid_tasks` with the evaluator's message.

A precondition is authored over HTTP: `POST /task` and `PATCH /task/{id}`
accept `preconditions`, per [Task capture](TASK_CAPTURE.md). The app sends
none.

## The write

The result is stored through the store's `persist_universe_state` with an
envelope from `AppState::envelope_for`, `AuthoritySource::User`, observing the
version that was read. That is the path a completed Task's effects take. It
runs under the same lock as a Task action, so an edit and a completion do not
interleave.

`persist_universe_state` updates an existing row and refuses to create one. On
a store with no `UniverseState` the route therefore admits an empty one first
at version 1 and then writes the edit as version 2, as a completion does. The
first edit on an empty store answers `version: 2`, the next `3`.

## Compartment labels

`persist_universe_state` copies the label of the row it updates, so **an edit
keeps the label of the row it supersedes**. A row the route itself has to
create is labelled `user-capture`, the label every other object the operator
authors by hand carries (Objectives, Preferences, Settings).

That makes three ways a `UniverseState` row can be born, with three labels:

| Born from | Label |
|---|---|
| `POST /bootstrap/seed` | `bootstrap` |
| The first completed Task with effects | that Task's label |
| The first edit through this route | `user-capture` |

The labels are not normalised here. When Compartment enforcement is scoped,
this is one of the places it will bite: a Device without the row's Compartment
would lose sight of the current state.

`POST /bootstrap/seed` runs once, and it always admits a new `UniverseState`
with a new id. On a store that already has one from a completion or an edit,
the bootstrap row becomes current and what the older row held is no longer
read. This route does not change that.

## Bootstrap's keys

`POST /bootstrap/seed` stores five values: `operator.work_style`,
`operator.attention_preference`, `project.repository` and `project.objective`
in `facts`, and `operator.planning_horizon_days` in `numeric_values`. Their
targets are `facts.operator.work_style` and so on.

Until P1B-59 each key began with its collection, `facts.operator.work_style`
inside `facts`, so the target was `facts.facts.operator.work_style`. That was
more than untidy. `ubu-core` reads the segment after the collection as a
target's namespace, and refuses the `affect` namespace outside `user_mode`.
Under the doubled convention an intrinsic-affect fact would have been targeted
`facts.facts.affect.x`, its namespace would have read as `facts`, and the
guard would not have fired.

**There is no migration.** A store bootstrapped before P1B-59 keeps its
doubled keys, and nothing reads them by name. The operator's store held no
`UniverseState` when this changed.

## What it does not do

- **`captured_at` and `source_summary` are not rewritten by an edit**, as they
  are not by a Task's effects. The row's `updated_at` and its envelope record
  when and by what authority it last changed, and `fact_provenance` records it
  for each value.
- **It authors no precondition.** A Task's `preconditions` are written through
  the Task routes.
- **It has no advisor.** Nothing proposes a precondition or a fact. `proposed`
  is a kind a mutation may state, and nothing here states it.

## Tests

`tests/universe_state.rs` holds the route's contract, including a Task that a
precondition blocks until the route records the fact and blocks again when the
fact is cleared, a Task that waits on a number being `at_least` a value, a
number set to exactly the value asked for, and the provenance a write records
and a clear removes. `tests/bootstrap.rs` holds the undoubled keys. `src/services/universe_state.rs` holds the tests that need the
planner's own reader or a mode other than this instance's.
