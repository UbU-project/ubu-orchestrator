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
names as `facts.operator.work_style` is stored under `operator.work_style` in
`facts`. A target is a collection, subject, optional entity path and final
predicate. `fact_provenance` is keyed by the complete target.

## Edit

`PATCH /universe-state` takes

```json
{
  "schema_version": "ubu.orchestrator.universe_state.v1",
  "mutations": [
    {"operation": "set_fact", "target": "facts.operator.synthetic_ready", "payload": true}
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
   `user_mode`. The MVP instance is `user_mode`; the manual route's additional
   namespace rule below also applies in this mode.
2. The manual route refuses a write whose first key segment is `facts`,
   `numeric_values`, `set_memberships`, `event_markers` or `affect`.
   This covers `set_fact`, `set_numeric`, `increment_numeric`,
   `decrement_numeric`, `add_membership` and `append_event_marker`.
   The four collection names repeat a collection inside the key; `affect`
   names intrinsic affect, which core treats specially and the other modes
   refuse. The collection comes from the panel, not the key.
   Only the first key segment is checked: `facts.kettle.affect.note` and
   `facts.fact.kettle` remain valid. Task effects keep their core contract.
   Existing keys are not migrated or refused on reads, advisor vocabulary
   enumeration or precondition evaluation. `clear_fact`, `clear_numeric`
   and `remove_membership` remain available for legacy keys.
3. P1B-69 checks each new write for a subject and a predicate, a subject in
   the effective vocabulary, a lowercase snake_case final predicate, ASCII
   entity-path segments and a complete target of at most 128 bytes. This
   applies to the same six write operations; clear/remove retains legacy
   semantics. The registry is read under the same Task-action lock that
   serializes its minting/retirement. No failed list leaves a seed or partial edit.
4. `apply_universe_mutations` on the current state, or on an empty one when the
   store holds none. It validates the whole list before applying any of it.
5. Only then the write.

So a list with one bad mutation is refused whole. No earlier mutation in it is
applied, and on a store with no state no empty row is left behind.

| Status | Code | When |
|---|---|---|
| 400 | `missing_schema_version`, `unknown_schema_version` | The request names no version or another one. |
| 400 | `universe_mutations_empty` | `mutations` is an empty list. |
| 400 | `universe_mutation_mode_invalid` | Step 1 refused. The message is `ubu-core`'s. |
| 400 | `universe_target_namespace_invalid` | Step 2 refused. The message names the reserved segment and complete target. |
| 400 | `universe_target_subject_unknown` | The subject is outside the effective vocabulary; the message names explicit minting. |
| 400 | `universe_target_grammar_invalid` | The new write lacks a subject/predicate or violates their mechanical grammar. |
| 400 | `universe_mutation_invalid` | Step 4 refused. The message is `ubu-core`'s, for example `mutation 1: unknown operation ...`, counting from zero. |
| 409 | none | The state changed between the read and the write. |
| 422 | none | The body is not this shape, for example a mutation with a key the type does not have. |

The namespace diagnostic has two message templates, with `segment` and
`target` replaced by the refused first key segment and complete target:

```text
Key segment `{segment}` names a collection; the collection comes from the panel, not the key. The target would be `{target}`.
Key segment `affect` is reserved for intrinsic affect, which organization_mode and worker_mode refuse. The target would be `{target}`; the collection comes from the panel, not the key.
```

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
  worked before P1B-59 keeps its provenance meaning; P1B-65 adds the manual
  route's namespace restriction above.
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
accept `preconditions`, per [Task capture](TASK_CAPTURE.md). The Tasks screen
reads them in words, authors one leaf and clears with null. UniverseState
itself authors no precondition.

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

## Keys and the open vocabulary gap (P1B-65)

At P1B-65 the screen began showing the complete target in `<code>` under each Facts,
Numbers and Sets key field, using the same trimmed key as the submitted
mutation. Event markers have no editing field. For example, `kettle.descaled`
under Facts shows `facts.kettle.descaled`; `fact.kettle` shows
`facts.fact.kettle`. The singular `fact` is deliberately not a reserved
segment. `affect.energy` previews the full target before the route refuses it.

The manual route's five reserved first key segments are a narrow grammar
restriction. A later segment named `facts` or `affect` remains permitted:
core reads the segment immediately after the collection as the namespace.
No migration runs, and existing doubled or reserved keys remain readable,
offered by the precondition advisor's `targets()` and evaluable by core.
Only the six write operations are restricted; legacy clear/remove remains
available. Task effects and core mutation semantics are unchanged.

UBU-D0243 in `ubu-design/DECISIONS.md` requires the first segment after the
collection to belong to its controlled subject vocabulary. At P1B-65 that broader
rule remained unenforced, because reasonable fresh-store subjects had no
explicit minting path. P1B-69 closes that authoring gap through the provisional
Setting registry and subject selector, preserving all legacy reads/evaluation. The five reserved
segments address collection repetition and intrinsic-affect semantics;
they were not a subject vocabulary advisor or a predicate naming rule.
P1B-69 adds the effective-vocabulary and final-predicate checks above, without
claiming semantic noun recognition or ratifying provisional roots.

The route contract was documented with section A's implementation because
this document requires a contract change in the same commit. This section
adds the UI and design context in section D; it changes no route behavior.

## What it does not do

- **`captured_at` and `source_summary` are not rewritten by an edit**, as they
  are not by a Task's effects. The row's `updated_at` and its envelope record
  when and by what authority it last changed, and `fact_provenance` records it
  for each value.
- **It authors no precondition.** A Task's `preconditions` are written through
  the Task routes.
- **Review may suggest a target name; only the operator supplies its value.**
  The vocabulary producer proposes names in facts or numeric_values, without
  any value. Admission dispatches set_fact or set_numeric through this service's
  shared mode, namespace and core mutation validation, exactly the checks the
  screen uses. The operator's supplied value is recorded as asserted.
  `proposed` provenance remains unused by advisors: it describes a suggested
  value, and no advisor suggests values. A separate precondition run can then
  propose a requirement over the newly recorded names.

## Subject ratification agenda (P1B-76)

The existing UniverseState **Subjects** panel is extended, not newly introduced.
It already listed the effective vocabulary and provided explicit mint/retire
controls with pre-request root validation. The ticket's claim that the subject
list diagnostic was dangling is withdrawn: that list existed. The diagnostic
and the governed vocabulary are unchanged.

`GET /settings` remains the registry source; no subject route is added.
Effective provisional `universe.subject.<root>` rows whose value is `true`
gain derived `subject_metadata`, containing `minted_at` from the Setting's
creation time and `references` with the three counts below. Their existing
Setting version remains the registry version. Ordinary Settings have no subject
metadata. These are response projections, not new canonical records or fields
on `ubu-core::Setting`. Canonical schemas and schema versions do not change;
the local OpenAPI response description and its UI copy are regenerated.

The client derives two named tiers from the existing list: the fixed governed
set (`operator`, `project`, `github`, `affect`, `relationship`) and the effective
provisional registry. The provisional rows show minting time, version and counts,
not reference keys, values or target strings. Its sentence computes
`UBU-D0291` satisfaction from the provisional tier: empty is satisfied **for
now**; otherwise ratification is outstanding and this screen is the agenda.
The condition is evaluated at the switch, not banked. This reports a condition;
it does not enforce the switch or discharge the separate planner gate.

| Reference count | Computation |
|---|---|
| `universe_state_keys` | Count each matching subject's key in `facts`, `numeric_values`, `set_memberships` and `event_markers` across every current stored UniverseState object row, including a non-latest row. Count keys, not values, members or marker entries. |
| `fact_provenance_keys` | Count full target keys in those rows' `fact_provenance` whose collection is recognized and whose subject matches exactly. |
| `task_precondition_targets` | Walk `payload.preconditions` in every current Task object row, including inactive Tasks, following `all_of` and `any_of`; count each matching target occurrence, including repetitions. Do not search descriptions, effects or `expected` values for lookalike strings. |

Subject matching uses the namespace segment, never a substring. Historical
object-history rows, Logs and candidate payloads are outside these three named
key spaces. Counts use one shared payload scan for all roots, not a separate scan
per root. The nested Task-target count has no indexed key column: it scans and
parses all current Task payloads. Work is linear in the stored payload bytes
and condition nodes; the current implementation fetches matching Task and
UniverseState payloads together, so memory includes those JSON strings and the
largest decoded payload. No new index, cache or migration is introduced.
Unreadable payloads/collections fail counting closed without exposing contents.

`DELETE /setting/universe.subject.<root>` now refuses with HTTP 409 and the
named `subject_referenced` diagnostic while any count is nonzero. The message
contains the three counts and the no-cascade remedy, never reference content.
The count check and DELETE share a SQLite transaction under the existing
import/Task-action locks. Retirement neither clears facts/provenance nor edits
Task requirements; the operator clears removable references first. Only three
zero counts permit retirement. The UI disables retirement with a reason, refreshes
counts after UniverseState edits and registry changes, and refreshes again on
a stale retirement refusal. Counts unavailable to the client disable retirement.
The former panel sentence permitting retirement while references remain is
replaced in the same UI change as these refusal controls.

**UBU-D0291 append-only-marker retirement gap.** Event markers have no clearing
operation. A root referenced by an append-only marker cannot take the ordinary
retirement leg of D0291's promoted-or-retired binary. It remains registered
pending operator ratification or separate cleanup work; this ticket does
neither and files no new decision. Other retained rows without an exposed
cleanup path also remain counted rather than hidden. No cascading erasure or
automatic promotion is a remedy. No root or governed-set extension is proposed.

No new Compartment scoping is added. The first-run Device's empty allowlist is
unchanged; no grant is needed or added by this view.

## Tests

`tests/universe_state.rs` holds the route's contract, including a Task that a
precondition blocks until the route records the fact and blocks again when the
fact is cleared, a Task that waits on a number being `at_least` a value, a
number set to exactly the value asked for, and the provenance a write records
and a clear removes. `tests/bootstrap.rs` holds the undoubled keys. `src/services/universe_state.rs` holds the tests that need the
planner's own reader or a mode other than this instance's.


## P1B-67: atomic admission of a suggested name

The mutation preparation in this service is shared by the screen and
record_universe_target admission. Both call validate_mutations_for_mode,
the same five-reserved-segment guard, and apply_universe_mutations before writing.
Admission additionally refuses a name already recorded, so it never overwrites
an operator assertion. No missing value is defaulted, inferred or derived.

The existing atomic candidate admission writer commits the prepared UniverseState
and the linked candidate decision together. A rejected, deferred or stale
candidate cannot leave a partial world write. Task and existing UniverseState
versions are observed, and the Task-action lock covers preparation and commit.
The evidence Task remains unchanged. For an empty store, this transaction creates
the complete populated state at version 1 with user-capture label. Existing
states retain their label, captured_at, source_summary and unrelated values.
The manual route remains unchanged: empty seed at version 1, edit at version 2.

This choice keeps store changes pin-only and avoids calling the manual route
first and admitting separately, which could leave a value or empty seed behind
if the candidate decision fails. It uses the existing writer, with shared
UniverseState service validation, and introduces no store logic or migration.
The fact_provenance entry is asserted because every value is the operator's
assertion, even when UbU suggested its name. P1B-69 adds effective-subject and predicate checks to new authoring, shared
with suggested-name admission. Plan predicates and Task effects are unchanged.


## P1B-69 C: new-write grammar

A target is <collection>.<subject>[.<entity-path>].<predicate>. New manual
writes require at least a subject and final predicate; the predicate is
lowercase ASCII snake_case starting with a letter. Optional middle segments
are nonempty ASCII letters, digits, underscores or hyphens. The full target
is at most 128 bytes. The effective subject set is the five governed roots
union true-valued provisional Settings named universe.subject.<root>, per
SETTINGS.md and UBU-D0291. The five reserved-first-segment refusals and their
existing reasons are checked before this additional grammar, including affect
in user_mode. Task effects and core's mode/evaluator semantics are unchanged.

Unknown subjects produce universe_target_subject_unknown. Malformed new targets
produce universe_target_grammar_invalid. Their exact templates are:

```text
Subject `{subject}` is not in the effective vocabulary. Mint it explicitly in UniverseState's Subjects list before writing `{target}`.
Target `{target}` needs a subject and a predicate: <collection>.<subject>[.<entity-path>].<predicate>, with a lowercase snake_case predicate, ASCII entity segments and at most 128 characters.
```

No migration or retroactive read/evaluation check runs. Previously authored
single-part, unknown-root, doubled or reserved targets remain stored, listed
in targets(), readable and evaluable. clear_fact, clear_numeric and
remove_membership still work on them. A subsequent write must meet today's
grammar; retiring a root does not rewrite old targets. Essential route contract
ships with this implementation; section F supplies the broader screen context.


## P1B-69 F: names the operator can author

A fact is a subject and a predicate, both the operator's words. Facts, Numbers
and Sets choose the subject from the effective vocabulary and type the final
predicate. An optional entity path is folded into that field before the
predicate, so a simple fact needs only two fields. Event markers stay read-only
in the app. The preview assembles the complete target; it has existed since
P1B-65, while P1B-68 added the assertion/reading choice.

The effective vocabulary is operator, project, github, affect, relationship
plus the provisional registry at Setting names universe.subject.<root>, with
boolean true. UniverseState's one Subjects list marks governed roots and
provisional roots awaiting ratification. Minting is a separate explicit act;
writing a value and every advisory producer lack minting authority. Affect
is visible but disabled for these manual forms, preserving P1B-65's refusal.

Minting mechanically checks lowercase ASCII snake_case beginning with a letter,
no dots, at most 64 bytes, no reserved or governed name and no duplicate.
The semantic half is stated at minting and judged by the operator: a singular
noun naming an entity or domain, never an instance, attribute, provenance/source
or reverse-DNS authority prefix. No plural or part-of-speech detector runs.
UBU-D0291 explains why every provisional root must be ratified or retired
before the switch, when the store stops being disposable. This ticket ratifies
none, promotes none and changes no design record.

An ungrammatical target authored before P1B-69 is kept, not rewritten. Reads,
targets(), the precondition enum and evaluation continue to include it. New
writes meet today's grammar; clear/remove works on old names. Even after
retirement, old targets stay readable/evaluable until explicitly cleared.
The precondition producer inherits new authoring's grammar through recorded
targets, while retaining legacy names; it changes no schema or validator.
