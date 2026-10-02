# The UniverseState route

A Task can ask that something be true before UbU will plan it. That something
is a `UniverseState`: four collections of facts about the operator's world. The
planner evaluates a Task's preconditions against it, and a completed Task's
effects change it. Until P1B-58 nothing let the operator see it or change it by
hand. `GET /universe-state` and
`PATCH /universe-state` do.

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
| `source_summary` | One sentence for the whole state. An edit does not rewrite it. |
| `confidence_summary` | One sentence for the whole state, or `null`. |

All four collections are always present, empty or not. A key is the part of a
target after its collection: the fact a precondition names as
`facts.kettle.descaled` is stored under `kettle.descaled` in `facts`.

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
`target`, and optional `payload` and `note`. The route defines no operation of
its own. The seven are the ones a Task's `effects` use:

| Operation | Collection | Payload |
|---|---|---|
| `set_fact` | `facts` | any JSON value, required |
| `clear_fact` | `facts` | none allowed |
| `increment_numeric` | `numeric_values` | a number |
| `decrement_numeric` | `numeric_values` | a number |
| `add_membership` | `set_memberships` | a JSON scalar |
| `remove_membership` | `set_memberships` | a JSON scalar |
| `append_event_marker` | `event_markers` | a JSON object |

There is no "set a number" operation. A number is moved to a value by
incrementing or decrementing it by the difference, and a key that is not there
counts from zero. There is no operation that removes a number or an event
marker.

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

## What it does not do

- **No per-fact provenance.** `source_summary` and `confidence_summary`
  describe the whole state. Nothing records that one fact was measured and
  another asserted. That needs a `ubu-core` field and is not invented here.
- **`note` is accepted and not stored.** The mutation type carries it, the
  applicator ignores it, and the state has nowhere to keep it.
- **`captured_at` and `source_summary` are not rewritten by an edit**, as they
  are not by a Task's effects. The row's `updated_at` and its provenance record
  when and by what authority it last changed.
- **Bootstrap's keys carry their collection twice.** `bootstrap_service` stores
  its facts under keys such as `facts.operator.work_style`, inside `facts`. The
  target that addresses one is therefore `facts.facts.operator.work_style`.
  The route reports the keys as they are stored.
- **It authors no precondition.** A Task's `preconditions` are written
  elsewhere. `evaluate_leaf_precondition` supports `equals`, `member_of` and
  `absent`.

## Tests

`tests/universe_state.rs` holds the route's contract, including a Task that a
precondition blocks until the route records the fact and blocks again when the
fact is cleared. `src/services/universe_state.rs` holds the tests that need the
planner's own reader or a mode other than this instance's.
