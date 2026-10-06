# Advisory proposals and Review

**Proposals never mutate canonical Task state. The advisor only enqueues
candidates; admission is an explicit operator act.** Model output is untrusted
input. The existing validation, durable candidate store and ordinary admission
path remain the boundary between a suggestion and a Task change.

## What P1B-45 connects

The advisory controller, transport trait, queue and candidate lifecycle predate
this ticket (UBU-D0274). What was missing was a real transport, an explicit
trigger, and a producer. P1B-45 supplies an Ollama implementation of the existing
`AdvisoryTransport`, `POST /advisory/run`, and one producer: `suggest_tags`.
Review is after Routines and before Calendar; configuration is beside Colours
in Setup. Nothing polls, schedules runs or invokes a model on app startup.

## Configuration and manual runs

Both `advisory.model` and `advisory.endpoint` are non-empty string Settings under
UBU-D0287. `GET /settings` includes an `advisory` array with `value` and `origin`
(`setting` or `unconfigured`) for both names. Save uses `PUT /setting/{name}`;
Revert uses `DELETE /setting/{name}`. Neither has a built-in default. From
P1B-46 a third Setting, `advisory.timeout_ms`, sets the budget and does have a
default; see [the three advisory Settings](#the-three-advisory-settings).
Other unknown names still produce `setting_unknown_name`.

The endpoint must be exactly `http://127.0.0.1:<port>`, with port 1–65535.
Hostnames, IPv6, HTTPS, userinfo, paths (including a trailing slash), queries and
fragments are refused. The transport appends `/api/generate`, disables proxies
and redirects, and does not discover or manage models. Model existence is known
only when a run succeeds or fails. Endpoint validation uses
`setting_invalid_advisory_endpoint`; empty/non-string values use
`setting_invalid_advisory`.

A run request is:

```json
{"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"suggest_tags","limit":5}
```

Omitting `limit` means 5; the accepted range is 1–25. Selection includes active
Tasks with no `category_tag`, ordered by Task ID. Static and Dynamic Tasks are
eligible, even if other tags exist. Completed and already-categorized Tasks are
excluded.

**Routine occurrences are excluded, from P1B-47.** An occurrence is rebuilt from
its routine's template at the next materialize, so a category admitted onto one
would be gone afterwards. Each uncategorised occurrence passed over is reported
with `suggest_tags_occurrence_skipped`, naming the Task and saying that a
routine's category belongs on its template, which the Routines screen edits. Up
to three are named one by one; any beyond that are counted in one further
diagnostic. These come first in the run's diagnostics, before anything the model
run reported. A skip is not a failure: the run's status is unaffected. The response names every selected ID/title, every created candidate
ID, the enqueued count, the validated returned proposals in `report`, and any
diagnostics. A Task may be omitted by the model; a successful empty proposal
array creates nothing. Repeated identical queued proposals are not duplicated.

An absent Setting returns HTTP 200 with `advisory_unconfigured` naming it and
performs no selection or model call. Connection, timeout, HTTP status, size and
malformed-result failures return failed-result diagnostics with no candidates,
not HTTP 500. Invalid request version, producer or limit is a request rejection.
No raw HTTP error body is returned or logged by the transport. From P1B-46 one
field of a refusal, the server's own `error` string, is echoed bounded; see
[what a failure says](#what-a-failure-says).

## Exactly what is sent

**Task IDs and titles are the only Task data sent to the model.** The minimized
submission payload is an array, for example this synthetic fixture:

```json
[{"id":"task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e70","title":"Synthetic lunar teapot 0"}]
```

This array becomes the JSON string in Ollama's `prompt`. The HTTP body also
contains the configured `model` name, `stream: false`, `think: false` (from
P1B-46), fixed system instructions and a fixed JSON output schema in `format`. It never serializes the submission
envelope into the prompt. No Task provenance, compartments, objective refs,
evidence, notes, time windows, Settings other than model name, device identity,
authority grants, digests or causal parents are sent. The endpoint is literal
loopback: data crosses to the local Ollama process, not to a remote model
endpoint. The implementation cannot attest to that separately operated process's
own behavior. This is a description of the orchestrator's outgoing request.

The wire format follows Ollama's documented
[`POST /api/generate`](https://docs.ollama.com/api/generate) with streaming disabled
and a structured output schema. Its completed response contains `done: true`
and a `response` JSON string of this shape:

```json
{"proposals":[{"id":"task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e70","category_tag":"work","confidence":0.8}]}
```

Only selected IDs, one proposal per Task, non-empty category strings up to 100
bytes without control characters, and confidence from 0 to 1 are accepted.
Unexpected proposal fields or malformed output refuse the entire result.
The model cannot provide candidate authority or change the operation: the pure
wire layer builds a Tag candidate with normalized `set_category`, the selected
Task reference, local evidence ref `<task-id>:title`, and the configured model
name as proposing actor. None of those additional candidate metadata are sent
back to the model.

The submission permits only Tag proposals and diagnostics. Timeout is
`advisory.timeout_ms`, 120 seconds unless set, and result size is bounded at
256 KiB, both while receiving bytes and after expanding candidate metadata.
Declared `ComputeBudget` is the same number of CPU milliseconds as the timeout,
and 512 MiB. These fields do not impose a CPU/RAM quota on the separately running
Ollama process. No runtime compiler or model concurrency/memory caps were added.

## Review and the durable lifecycle

`GET /advisory/queue` retains its active `candidates` list (proposed/resurfaced),
adds `deferred_candidates` for manual resurfacing and `target_titles` for display.
Each candidate shows kind, target, normalized proposal, confidence, evidence,
actor and age. An empty active queue is an ordinary state.

- **Admit:** `POST /advisory/candidate/{candidate_id}/admit`, with
  `observed_version`. Ordinary admission validates the current active Task,
  sets `category_tag`, and ensures that category is also a Task tag. A different
  category set since the proposal was made causes `advisory_category_changed`.
  Existing `add_tag` candidates keep their original tag-only semantics.
- **Reject:** the existing `/reject` route takes `observed_version`, `reason`
  and `retention_policy: "retain"`. Review asks for confirmation because this is
  durable. The controller checks durable suppression before enqueueing another
  candidate with the same kind, normalized proposal and target references.
  Repeated runs cannot bring that identical proposal back. A different category
  or differently-worded proposal is a different candidate, not a blocked pattern.
- **Defer:** `/defer` with `observed_version` keeps the proposal durable and
  moves it out of the active queue into Review's deferred list.
- **Resurface:** `/resurface` with `observed_version` and
  `trigger: "user_request"` returns a deferred proposal for review.

Each explicit action reloads the list. Existing lifecycle requests have no
schema-version field; Run uses `ubu.orchestrator.advisory_run.v1` and persisted
candidate schema version remains `1.0`. The existing candidate-detail GET route
also remains available, though Review does not need another client constant for it.

The live transport is executable-only under `cfg(not(test))`, not a library
export. Test states have no transport factory or fallback. Tests inject a stub
implementing the existing trait, exercise the pure wire policy, and structurally
check that the live implementation cannot be constructed through test wiring.

## The three advisory Settings

| Name | Value | Validation | When absent |
|---|---|---|---|
| `advisory.model` | string | Non-empty. Refused with `setting_invalid_advisory`. | No default. A run reports `advisory_unconfigured`. |
| `advisory.endpoint` | string | Exactly `http://127.0.0.1:<port>`. Refused with `setting_invalid_advisory_endpoint`. | No default. A run reports `advisory_unconfigured`. |
| `advisory.timeout_ms` | integer | From 5,000 to 3,600,000 inclusive. Refused with `setting_invalid_advisory_timeout`, which names the bounds. | **120,000.** |

`advisory.timeout_ms` is the budget for one whole run, in milliseconds. It
exists because two minutes is not enough on the machine UbU is built for: a
model with a large context window can need swap to process even a modest
prompt, so its first token can be minutes away.

- **The floor is five seconds and the ceiling is one hour.** A zero would make
  every run fail at once. No ceiling would let one run hold the advisory path
  indefinitely.
- **It must be a JSON integer.** `60000.5`, `"60000"`, `true` and `null` are
  refused. Nothing is admitted on a refusal.
- **`max_cpu_ms` is the same value.** The two describe the same run, so there
  is one knob. `max_memory_bytes` stays 512 MiB.
- **`GET /settings` reports it** as the third entry of `advisory`, with
  `origin` `setting` or `default`. Its `value` is the number of milliseconds
  as a string, such as `"120000"`, because the entry's `value` is a string for
  all three names. The admitted Setting in `settings` holds the number.
- **Reverting** with `DELETE /setting/advisory.timeout_ms` returns to the
  default.

The rejection reads:

```text
advisory.timeout_ms must be an integer number of milliseconds from 5000 to 3600000
```

## Why `think: false` is sent

The request body is now:

```json
{"model": "…", "stream": false, "think": false, "system": "…", "prompt": "…", "format": {…}}
```

`think: false` is sent on every request. The lesson is Quick UbU's. Its
working transport, `quick-ubu/ollama-planner/src/lib.rs`, carries the flag
under this comment:

> `// API equivalent of `ollama run --think=false`. Thinking output is`
> `// already omitted from the assembled answer (`--hidethinking`).`

Without the flag, a reasoning model generates a thinking block before the
schema-constrained answer. Mainline sends `stream: false`, so the client waits
for the whole generation, thinking included. The answer can then be empty,
with the thinking in the payload. Quick UbU's `completed_answer` reports
`thinking_present` for exactly that failure. Mainline inherited the endpoint
and not the lesson.

A non-reasoning model ignores the flag. The advisory path never wants a
thinking block, because it would discard it.

Nothing else in the body changed: the system prompt, the `format` schema and
`stream: false` are as P1B-45 left them.

## What a failure says

Three failures have three different remedies. They are kept distinct.

| Code | Means | Change |
|---|---|---|
| `advisory_http_failed` | The server answered with a status outside 2xx. | **The model name.** Most often the model has not been pulled. Check `advisory.model`. |
| `advisory_timeout` | The run did not finish within the budget. | **The budget.** Raise `advisory.timeout_ms`. |
| `advisory_empty_response` | The server answered 2xx and the answer was empty or whitespace. | **The flag or the model.** With `thinking_present: true` the model thought instead of answering: choose one that honours `think: false`, or a non-reasoning model. With `false`, run again or choose another model. |

`advisory_connection_failed` is a fourth: nothing answered at the endpoint.
Start the server or correct `advisory.endpoint`.

In every case no candidate is enqueued and the run answers HTTP 200 with the
diagnostic.

### The server's `error` field is echoed. Generated text never is.

When a refusal carries a JSON body with a string `error` field, that field is
included in `advisory_http_failed`:

```text
The local model returned HTTP 404: model 'x' not found; check advisory.model and that the model has been pulled; no candidates were enqueued
```

It is cut to 200 characters and control characters are removed. When the body
is not JSON, has no `error`, or `error` is not a non-blank string, the message
is the generic wording P1B-45 used.

These two kinds of text are different, and the difference is the point:

- **The `error` field** is a short message from a local service the operator
  configured and pointed the orchestrator at himself. It is read only from a
  response whose status is outside 2xx. No other field of that body is read.
- **Generated text** is untrusted content. It is parsed as a proposal or it is
  refused. It is never copied as free text into a diagnostic, a Log entry or a response.
  P1B-61 permits only bounded, syntax-checked target identifiers in a missing-target
  diagnostic, as specified below; expected values and descriptions are never echoed there.

### An empty answer is a failure

An answer that is present and blank is not a run that found nothing to
propose. It is reported as `advisory_empty_response` with status
`malformed_result`.

The diagnostic says whether thinking was present: `thinking_present` is in
the message, and is a boolean field of the diagnostic in `report`. **Only the
boolean is reported.** The thinking text is not read into any result,
diagnostic, Log entry or stored row.

A run whose answer is a valid, empty proposal list is still a success.

## A Dynamic Task's category produces no calendar colour

Admitting a tag proposal sets the Task's `category_tag`. Whether the Calendar
then shows a colour depends on the Task's placement.

`src/services/planning_service.rs` exports `gcal_color_id` only for a Static
placement:

```rust
gcal_color_id: if task.static_anchor {
    titles
        .get(&task.task_id)
        .and_then(|display| display.gcal_color_id.clone())
} else {
    None
},
```

This is P1B-33's partition, described in
[Calendar interaction](CALENDAR_INTERACTION.md). On a Static event the colour
is its category. **On a Dynamic event a colour means done**, so a category
must not produce one. The category resolves through the palette and placement
withholds it.

So after admitting a category for a Dynamic Task, `category_tag` is set on
the Task and the Calendar preview shows no colour. That is correct. Review
says so beside the proposal, before the decision.

To see a category colour, admit a proposal for a Static Task: a routine
occurrence, or a Task with a fixed window.

## Clarify: the interview

From P1B-48 there is a second producer, `clarify`. It interviews one Task, to
turn a short capture into something plannable. See
[the interview](CLARIFY.md) for the whole of it. In short:

- A run proposes **one candidate for one Task**, a set of questions.
- **Answering it is what admits it.** `POST /advisory/candidate/{id}/answer`
  writes the questions answered, and the answers, to the Task's `description`.
  Plain Admit refuses a clarification proposal with
  `advisory_answer_required`.
- **The prompt carries the Task's `description`**, which holds the answers
  already given. This is wider than SuggestTags, which sends ids and titles
  only. It goes to the operator's own configured loopback model and nowhere
  else.
- The three advisory Settings, the timeout, `think: false` and every failure
  described above apply to `clarify` unchanged.

## Operator acceptance (live local Ollama; not performed by automated tests)

1. In Setup, save the model name and literal loopback endpoint. Try a non-loopback
   endpoint and confirm the refusal, then restore valid configuration.
2. With Ollama stopped, press Run in Review and confirm the diagnostic and zero
   new candidates.
3. Start Ollama yourself and capture two or three synthetic uncategorized Tasks.
4. Press Run; verify the selection and created candidates by name and ID.
5. Admit one; verify its Task category. For the Calendar colour check use a
   **Static** Task and a mapped category: the existing Static/Dynamic colour
   partition is unchanged, and Dynamic events remain uncoloured.
6. Reject another, confirm durability, and run again. The same normalized
   proposal for that Task must not return. A different proposal may still arrive.

## Later producers and remaining limits

`Advise` and `Batch` are not implemented. `Clarify` is, from P1B-48. Advise
would propose richer advice; Batch would provide unattended scheduling/batching. Each needs its own selection,
authority and disclosure decision. SetModel is configuration, not another screen.

Runs remain manual, payloads remain IDs/titles only, endpoints remain loopback,
and there is no model installation or management: whether a configured model
exists is still discovered when a run fails, though the failure now names it.
`stream: false` is unchanged, so the operator waits for the whole generation
with no progress indication. The timeout is one value for all producers. Rejection suppresses exact
candidates, not patterns. Uncaptured recurring commitments still occupy no
planning capacity; see the temporary **Busy** blocking-event workaround in
[Calendar bootstrap](CALENDAR_BOOTSTRAP.md). Calendar export still records no
approver, and the Routines screen still cannot clear an override.

## P1B-61: propose a precondition

`POST /advisory/run` accepts `producer: "precondition"` and the existing optional
`limit` (default 5, maximum 25). It selects active, non-occurrence Tasks with a
non-blank title or description, ordered by ID, including those with an existing
precondition. Skipped Tasks are diagnosed, up to three individually and the rest
counted in one further line. The candidate
shows what explicit admission would replace; proposing changes no Task.
An empty vocabulary gives `precondition_no_facts` without a model call or write.

The model receives Task IDs, titles with or without descriptions, and the current
supported UniverseState target names, not its values or provenance. From P1B-64,
the response schema offers only predicates that vocabulary supports. `equals`
and `absent` use the whole vocabulary; `member_of` uses only `set_memberships`
targets; the four comparisons use only `numeric_values` targets. An empty
partition contributes neither its branch nor its predicates. Facts alone offer
exactly `equals` and `absent`. The partition is derived from target prefixes,
never configured. Each leaf's grammar requires or forbids `expected`:
`absent` forbids it; equals and membership require a string, number or boolean;
comparisons require a number. Null is excluded.
From P1B-63, a missing or whitespace-only description is omitted from the model
payload. The title is then the whole known account of the work. A title is on
screen; the earlier described-only gate made this producer unreachable on a
store built from calendar capture. Calendar notes may now supply a description,
but notes and an interview remain optional. Titles, descriptions and target names
are data, never instructions. All fixtures are
invented. A response has `proposals: [{id, precondition}]`; the normalized
candidate proposal is the tree itself for a Task with no prior precondition.
For a replacement it is `{existing_precondition, proposed_precondition}`, both
trees. The controller reads the existing tree from canonical state after model
validation; it never trusts the model to supply it and does not send that tree
to the model. Both trees participate in durable duplicate/suppression identity.
Candidate target_refs names one Task.
Evidence refs always name `<task-id>:title`, and also
`<task-id>:description` when that field was supplied to the model. They name
input fields, not a claim that the model's hidden reasoning used each field.

The controller validates before enqueueing, even for injected transports. It
checks the instance mode, strict tree shape, every leaf with core's evaluator,
and the whole tree. Every branch is checked despite boolean short-circuiting.
Trees are bounded to 128 nodes and depth 16. A refused proposal gives
`precondition_proposal_refused` and loses only its own candidate; surviving
proposals remain and the run stays `ok`, even when every proposal is refused.
The diagnostic explains the refusal with code-authored text, naming up to three
Tasks, then counting further refused Tasks without identifiers or targets.
Diagnostics already recorded in the run are preserved. Tasks that become
ineligible at the controller recheck follow the same three-named-then-count
rule. `advisory_malformed_result` means the response could not be decoded as a
proposal result at all, rather than that one tree was refused. Existing
transport failures and oversized-response rejection retain their codes and
statuses. Valid trees with
missing targets give one information diagnostic per Task for the first three,
then one count of further Tasks without ids or targets. No candidate is enqueued
for those Tasks. Repeated identical proposals do not duplicate the durable queue.

Only target identifiers of at most 128 ASCII bytes, with a recognized collection
and nonempty dot-separated alphanumeric/underscore/hyphen segments, may appear
in `precondition_missing_targets`. The first three are named and remaining
unique names counted. Arbitrary model text, expected values and descriptions
are not copied into this diagnostic. This is the operator-approved narrow
exception to the generated-text rule above, not permission to echo model prose.

The safety restriction is stronger than predicate evaluation: every leaf must
name an existing target, including `absent`. Contrary to the ticket's original
explanation, `absent` is true for a missing target; numeric comparisons are false.
A missing fact can later be authored, so blocked does not mean blocked forever.
The advisor changes neither rule. It writes only through candidate enqueue,
including its ordinary mutation-envelope metadata. It authors no facts and no
`proposed` provenance, and it does not move Tasks or write Task preconditions.

### The schema is a conservative contract (P1B-64)

Every tree admitted by the model's format schema must pass the unchanged
validator against the matching current vocabulary in this user-mode instance.
Every tree refused by that validator must also be refused by the schema. This
is a safe subset contract, not equality of the two accepted sets. The validator
continues to check strict shape, non-empty groups, every leaf, instance mode,
whole-tree evaluation and existing targets before enqueue or admission.

The operator approved three corrections to the ticket's proposed contract:

1. **Keep the validator and describe the schema as a subset.**
   `UNIVERSE_STATE.md` permits facts containing any JSON value, and core equality
   can evaluate object/array expectations. The existing validator accepts them.
   Scalar-only output and shallower trees therefore cannot describe exactly the
   validator's set. Restricting the validator would break existing authoring and
   admission; expanding the grammar to every supported JSON expectation would
   enlarge the small model's grammar and withdraw the ticket's scalar-only
   choice. Keeping the established evaluator and testing the safe-subset
   relationship preserves both contracts honestly.
2. **Use three levels and ten children per group.** A root group may contain
   groups of leaves; a leaf may also appear at either earlier level. Root depth
   is zero, so the deepest model leaf has depth two. There are no recursive
   schema cycles. The largest tree has `1 + 10 + 100 = 111` nodes, inside the
   validator's 128-node bound. Limiting depth alone while retaining 128 children
   would admit oversized trees. Fully expanding to depth 16 or enumerating
   combinations to use all 128 nodes would make the grammar much larger without
   making an operator's condition easier to read. The conservative limit is
   explicit; the validator's depth 16 and 128 nodes remain unchanged.
3. **Make the prompt's predicate count truthful.** “The seven allowed
   predicates” is replaced by “the predicates allowed by the response schema.”
   Keeping seven would contradict a facts-only format with two predicates;
   rewriting the whole prompt would needlessly alter its other constraints.
   Only this approved phrase and the two now-redundant collection/expected
   sentences change; every remaining sentence is preserved.

The representative tree table tests both schema membership and validation,
including equals without expected, comparisons over facts, the intentional
non-scalar/depth/breadth subset exclusions, and the 111/131-node boundaries.
Schema tests interpret the built JSON Schema keywords independently of
precondition semantics, without a new dependency or process. Refusal reasons
separate bounds, group shape, leaf fields, expected presence, decoding, null,
mode and evaluator errors. Core evaluator messages are taken only after target
and predicate validation; they identify the required kind, never the supplied
expectation. No model prose, expected value or description is copied into a
refusal diagnostic. No core predicate, admission rule or vocabulary advisor
is added.


### Bounded proposal labour (P1B-66)

A run still considers up to 25 Tasks; `MAX_LIMIT` is unchanged. The model's
`proposals` array is bounded to three (or the number selected if smaller).
The system text states that total bound, and the wire rejects four or more
proposals whole through the existing `advisory_malformed_result` path, even
when their trees are valid. Tree grammar and per-proposal validation are unchanged.

Before settings, selection or any `advisory_transport_factory` access, a
precondition run counts only `precondition` candidates in `proposed` and
`resurfaced` states. Ten or more produces HTTP 200, status `ok`, no selection,
no report and no enqueue, with `precondition_queue_full`:

```text
{count} precondition candidates are waiting in Review; review, defer or reject them before asking for more. No model was asked.
```

The count is the current number waiting, not a fixed ten. Deferred candidates
are excluded because setting work aside must free capacity for another run.
Other candidate kinds are excluded. Nine awaiting permits a run. This is a
refusal threshold, not a strict queue-size ceiling: a permitted run can add
three to nine and leave twelve for review. It changes no candidate lifecycle
and does not automate admission, rejection, deferral or resurfacing.

Ten is a judgment about the operator's attention, not a measured optimum;
the operator may want it lower. The model chooses which three Tasks to propose
for, and a weak model may choose the first three. Neither cap improves any
proposal's relevance. The small hand-authored vocabulary leaves the model
using the same few facts for unrelated work; P1B-67 addresses that vocabulary
cold start. This ticket makes the queue reviewable while that gap remains.
The existing diagnostic sentences and finite review snoozes are unchanged.

### Admission of a precondition

Admit uses the existing candidate route and atomic ordinary admission writer.
The kind identifies the set-preconditions action. A bare tree expects no prior
precondition; a replacement pair expects exactly its reviewed existing tree.
Admission sets only `Task.preconditions` to the proposed tree,
preserving placement and all other Task fields. It revalidates against current
UniverseState and observes both the Task and UniverseState versions in its
mutation envelope. A target cleared after proposal therefore refuses admission.
No UniverseState, fact or provenance is written by admission.

A Task whose precondition differs from the reviewed prior state is refused with
`advisory_precondition_changed`, including a condition added, changed or cleared
while the candidate waited. An unchanged prior tree may be explicitly replaced.
A second admission is refused (409), not applied twice. Reject, defer and resurface use the existing
lifecycle, and a deferred candidate cannot be admitted until resurfaced. The
next generated Plan evaluates the admitted precondition against current facts.

## P1B-62: review an admitted precondition

`producer: "precondition_review"` uses the same manual run route. Active,
non-occurrence Tasks with a precondition are eligible, including a Task with an
empty description. The model sees each Task's ID, description, current requirement
in exactly the words of `PreconditionWords`, and supported target names. It sees
no fact values, other Task fields or Log. Each Task receives one verdict:
`sound` produces only an aggregate count diagnostic; `replace` and `remove`
produce candidates with a nonblank model reason and the existing tree.
Replacement also carries a strictly validated proposed tree; removal has none.
`blocked_now` records a false evaluation at review time. No confidence is requested.
Generated reasons are candidate content, never diagnostic text. Proposals change
only the candidate queue and its ordinary enqueue mutation metadata.

Review admission dispatches `replace_precondition` and `clear_precondition`
explicitly. Both use the ordinary atomic candidate admission writer and compare
the Task's current condition with the reviewed tree. A stale tree or a second
admission is refused with 409. A deferred review must resurface first.
Replacement revalidates every proposed leaf against current facts and observes
Task and UniverseState versions. Removal clears the field, observes the Task
version, and needs no fact or proposed-tree validation: no guard remains.
Neither operation changes any other Task field or authors a fact.

Review dismissal policy is specified by [Admission review](ADMISSION_REVIEW.md).
The queue includes `review_intervals` keyed by candidate ID: suggested days,
seed, ceiling, blocking cap, evaluation time, return date and saved hold date.
Review defer/reject accept optional `snooze_days`. Settings
`advisory.review_seed_days` (default 7) and `advisory.review_ceiling_days`
(default 365) are whole days with `1 <= seed <= ceiling <= 365`, beside model,
endpoint and timeout. Generic proposal suppression retains its behavior.
Review operations use subject keys, event-based escalation and finite holds.
`force: true` on a `precondition_review` run explicitly reconsiders holds;
other producers refuse the field. Ordinary review runs honour holds.


## P1B-67: vocabulary names, operator values

producer: vocabulary uses the existing POST /advisory/run and limit (default 5,
1–25). Selection reuses precondition's active, non-occurrence, title-or-description
gate and Context. The model sees only Task IDs, titles, optional descriptions
and existing target names. No UniverseState observation, value, provenance,
Plan, Log or another Task's precondition is sent. Empty vocabulary is supported.
The password-hygiene agent receives none of this input.

The response format permits at most three proposals, including different names
for the same Task. The independent backlog counts only universe_target rows in
proposed or resurfaced lifecycle states. At ten or more, vocabulary_queue_full
returns ok with no selection or model call, before consulting configuration or
constructing a transport. Ten precondition candidates do not block Vocabulary;
deferred candidates of either kind do not occupy its backlog.

Each proposal contains id and target only. UniverseTarget candidates carry
record_universe_target and a name-only inline payload. A proposed value is
refused before it can become candidate content. target_refs names exactly one
Task: the evidence the operator judges, not the object admission writes. This
is the first advisory kind whose admission writes an object it does not reference.
Evidence refs identify title and optional description input fields, without
copying either into candidate metadata. The default suppression key over
{candidate_kind, normalized_proposal, target_refs} makes rejection durable for
that name and Task; another name or another Task remains a different subject.

The controller rechecks injected transports too. Target names are at most 128
ASCII bytes, with facts or numeric_values prefix and nonempty dot-separated
letter/digit/underscore/hyphen entity segments and a lowercase snake_case
final predicate. The subject is from the current effective vocabulary minus
affect; the request schema constrains that same grammar. Existing names and first key
segments facts, numeric_values, set_memberships, event_markers and affect are
refused by code. The request schema's positive pattern does not replace state, envelope or
selected-evidence checks in this validator. The five-reserved-segment helper is shared with the manual editor.

vocabulary_proposal_refused uses this exact diagnostic template:

```text
Task `{id}`: {reason}. No candidate was enqueued for this Task; the rest of the run stands.
```

If no selected Task can be named, the subject is A proposal. Reasons are:

- a proposal must contain only a target name; values belong to the operator
- the proposal must reference exactly one selected Task
- the target name exceeds 128 bytes
- the target requires a collection, subject, optional ASCII entity path and lowercase snake_case predicate
- the subject is outside the effective vocabulary; mint it explicitly in UniverseState's Subjects list
- only facts and numeric_values target names are in scope
- the first key segment names a reserved collection or intrinsic-affect namespace
- the target name is already recorded; an existing value must not be overwritten
- the Task is absent, inactive or a routine occurrence

At most three Tasks are named, then one count of further refused Tasks. Existing
result diagnostics are pushed to, never replaced. One bad proposal costs one
candidate; survivors and ok status remain. Four wire proposals or an undecodable
response give advisory_malformed_result with no candidates. vocabulary_task_skipped
explains the reused selection gate. vocabulary_no_task means no eligible Task,
no model asked. Transport diagnostics retain their existing behavior.

Admission on the existing candidate admit route accepts an optional value field.
For UniverseTarget its omission is refused as vocabulary_value_required:
“An operator-supplied value is required; no value is defaulted, inferred or derived”.
Explicit zero and false remain values; explicit JSON null is distinct from omission.
The name and active non-occurrence evidence Task are rechecked before writing;
name refusals use vocabulary_admission_refused with the reason above. Numeric
payload validation uses universe_mutation_invalid from the same UniverseState
service as the screen. Values on other candidate kinds are refused with
advisory_value_unsupported: “Only a target-name proposal takes an operator value”.

record_universe_target dispatches set_fact or set_numeric with asserted provenance.
The shared UniverseState service prepares the mutation; the existing atomic
candidate admission writer records the world and decision together. target_refs
is evidence, so the response keeps task as that unchanged evidence and adds
universe_state as the written destination. No value is inferred from model output.
FactProvenance::Proposed remains unused by advisors. Empty-store admission creates
one populated version 1 atomically; the manual editor keeps its version-2 first
edit. See UNIVERSE_STATE.md for that writer choice and concurrency observations.

The intended sequence is two explicit clicks: run Vocabulary, agree to useful
names and supply every value during admission, then run Precondition against the
larger vocabulary. Neither promises candidates. Vocabulary itself authors no
precondition. The existing precondition prompt, schema and bounds are unchanged,
as are finite review snoozes, Plan, capture, colours and collision diagnostics.


## P1B-68 C: shared selection notes

The active, non-occurrence, title-or-notes gate belongs to selection, not to
a producer. Vocabulary and Precondition each report its decision once under
advisory_task_skipped. Both remain separate explicit requests: no combined run
or cross-request suppression state is added. Review presents the latest shared
selection notes once instead of repeating them under both result panels.

The first three skipped Tasks use one of these exact messages:

```text
Task `{id}` is a routine occurrence; edit its template instead
Task `{id}` has neither a title nor a description to reason over
{N} more Tasks were skipped: they are routine occurrences or have neither a title nor a description
```

The final count names no Task. A store with eight ineligible Tasks produces
three named selection notes and one five-more note, not one copy per producer
in the rendered Review results. Each independent API response retains its own
gate decision. Missing-target, malformed-proposal, queue-full and late controller
refusals retain their producer-specific codes and behavior. No model input,
producer grammar, advisory authority, canonical mutation or queue bound changes.


## P1B-69 D: a model can never mint a subject

Vocabulary's request includes only effective subject names (governed plus
provisional), excluding affect. Its target pattern permits facts/numeric_values,
then exactly one of those subjects, optional ASCII entity segments, and a
lowercase snake_case final predicate, at most 128 bytes. The controller checks
the same name grammar and the intersection of the request's subject snapshot
with the current registry, so retirement or an injected result cannot bypass it.
Admission rechecks the current vocabulary. No fact observation or Setting value
is sent to a model; true is registry storage, not model context.

Affect remains governed but reserved for intrinsic affect and has mode
consequences under D0242. It is absent from Vocabulary's subject pattern; the
manual route preserves its existing reserved refusal. The system text drops
the collection instruction now enforced by the schema; it retains relevance,
name-only, existing-name and value prohibitions. One bad proposal still loses
one candidate, with vocabulary_proposal_refused, three named then a count.

Schema/validator equivalence here means the static new-name grammar. Existing
names, selected/active evidence, envelope shape and later admission state keep
additional checks; the request cannot guarantee future state. P1B-64's
conservative precondition grammar and validator remain unchanged. That producer
still enumerates recorded targets, including legacy ungrammatical targets; it
does not claim all stored names were authored under this ticket's grammar.
