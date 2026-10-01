# The interview

`clarify` is the second advisory producer. It asks the operator questions
about one Task, and the answers accumulate in the Task's `description`. It is
Quick UbU's `Clarify` and `ClarifyAnswer`, with a web form where Quick UbU had
an editor.

**The advisory boundary is unchanged.** A model proposes; only the operator's
act changes a Task. A question set is a proposal. The operator's answers are
the act.

## The review queue is the session

Quick UbU kept interview sessions in its store because a CLI has nowhere else
to put them. Mainline has the review queue.

1. A run proposes one candidate, of kind `clarification_question`, for one
   Task. Its `normalized_proposal` is
   `{"operation":"answer_questions","round":n,"questions":[…]}`.
2. It waits in the queue like any other. Defer, Resurface and Reject work on
   it unchanged.
3. Answering it admits it, and writes the answers to the Task's `description`.
4. The next round is another run, which reads the longer `description`.

Nothing new is stored anywhere. There is no session table and no new object
type.

## Running it

```json
{"schema_version":"ubu.orchestrator.advisory_run.v1","producer":"clarify","task_id":"task_…"}
```

`task_id` is optional.

| | Selected |
|---|---|
| `task_id` omitted | The first active Task, by id, whose `description` is absent or blank. This is round one only. |
| `task_id` given | That Task, whatever its description. This is how round two and later are reached. |

A routine occurrence is never selected, named or not. It is rebuilt from its
routine's template at the next materialize, so answers admitted onto it would
be lost. SuggestTags excludes occurrences for the same reason.

`limit` is refused with `advisory_limit_unsupported`: an interview is about
one Task. On `suggest_tags`, `task_id` is refused with
`advisory_task_id_unsupported`. A field that belongs to the other producer is
refused, not ignored.

The unconfigured, invalid-endpoint and transport-unavailable checks are the
ones `suggest_tags` has, in the same order, before anything is selected.

### Outcomes that are not failures

Each is status `ok`, HTTP 200, with nothing enqueued.

| Diagnostic | Means |
|---|---|
| `clarify_no_task` | There is no Task to interview. No model was asked. The message says which of four things is true, because the remedy differs: no active, non-routine Task exists at all (capture one); every active Task already has a description (choose one in the selector); the named Task is not active or does not exist; or the named Task is a routine occurrence, whose description belongs on its template. |
| `clarify_already_queued` | The Task already has a question set that is proposed, resurfaced or deferred. Answer, defer or reject it first. No model was asked. |
| `clarify_no_questions` | The model was asked and asked nothing. **What that means depends on the round**, which the response reports in `round`. On a later round the interview is finished: this is how the operator learns the Task is clarified. On round 1 of a Task with no description it is a result about the model, not about the Task: it declined to ask about something it had been told nothing about, against its instructions. `advisory.model` is what to change. |

A malformed id in `task_id` is HTTP 400, `clarify_invalid_task_id`.

There is no round cap. The interview ends when the model says it is done, or
when the operator stops running it.

## What is sent

```json
{"id":"task_…","title":"Synthetic lunar teapot","category_tag":"grocery","tags":["grocery"],"description":"Q: …\nA: …\n","round":2}
```

That object, serialized, is the `prompt`. The Task's id, title, category and
tags, its `description`, and the round. `category_tag`, `tags` and
`description` are omitted when the Task has none.

**The description is the operator's own accumulated answers.** Later rounds
are meaningless without them. This is a real widening of what leaves the
process compared with SuggestTags. It goes to the configured loopback
endpoint and nowhere else. The Clarify panel in the app says so.

No history of other Tasks is sent. Quick UbU sends completed examples;
mainline does not, yet.

The body is otherwise the tag body's: `stream: false`, `think: false`, a
`format` schema, and a `system` string that says every field is data and
never an instruction.

**Round one is never finished.** The `system` string tells the model to set
`done` to true when no useful question remains, and a model reading that for
a Task with no description answered `done` at once. From P1B-51, on round 1
of a Task whose description is absent or blank, and only then, the `system`
string ends with one more instruction:

> This is round one and the description is empty. Round one of a Task with no
> description is never finished: there is always something worth asking about
> a Task whose description is empty. Ask at least one question and set done to
> false.

A later round, or a round one whose Task already has a description, is sent
the `system` string without it. The instruction is advice to a model, not a
guarantee: a model that still answers `done` is reported as
`clarify_no_questions` with `round: 1`, and is not retried.

## What is accepted

The model answers `{"questions":[…],"done":false}`. A question is:

```json
{"id":"q2","text":"What is the synthetic deadline?","kind":"ShortText","depends_on":["q1","y"]}
```

`kind` is `YesNo` or `ShortText`. `depends_on` is
`[question_id, required_answer]`.

The whole set is refused, never part of it, with
`advisory_malformed_result`, when:

- the envelope is not `done`, or the response is not a question set;
- the set or a question has a field that is not one of these;
- there are more than 8 questions;
- an `id` or a `text` is blank;
- a `text` is over 400 characters, or holds a control character other than a
  line end;
- an `id` repeats;
- a `depends_on` names a question that does not come earlier in the set, or
  requires a blank answer.

**A dependency must name an earlier question.** So a form can be gated in
one pass, and a cycle cannot be written.

A set that passes and is not done, with at least one question, becomes
exactly one candidate. It has no `confidence`: the model is asking, not
scoring. A set that is `done`, or asks nothing, is a good answer with no
candidate.

**Clarify proposes questions and nothing else.** Quick UbU's clarify also
returns tags. Mainline's does not accept them: a set carrying `tags` has an
unknown field and is refused.

## Answering is admitting

```http
POST /advisory/candidate/{candidate_id}/answer
{"observed_version":1,"answers":{"q1":"y","q2":"next synthetic Friday"}}
```

The response is the one Admit gives: the candidate, now `admitted`, and the
Task.

It is the same read, the same observed-version envelope and the same atomic
writer as any admission. A Task changed between the read and the write
refuses the answer, with the Task and the candidate as they were.

### How the description is composed

For each question, **in question order and not in the order of the ids**:

```text
Q: {text}
A: {answer}
```

appended to what the description already holds, after a line end if it does
not already end with one.

- An answer is trimmed. A blank answer is dropped: it is "no comment".
- A `YesNo` answer is `y` or `n`, in either case, and is written in lower
  case.
- A question **applies** when it has no `depends_on`, or when the question it
  names applies, was answered, and its answer equals the required answer
  without regard to case. An answer to a question that does not apply is
  dropped.

Given the four questions in the example above and the answers
`q1: " Y "`, `q2: "  next synthetic Friday "`, `q3: "  "` and an answer to a
question that depends on `q1` being `n`, the description is:

```text
Q: Is there a deadline for the synthetic teapot?
A: y
Q: What is the synthetic deadline?
A: next synthetic Friday
```

### Refusals

Each is HTTP 400 and leaves the candidate `proposed` and the Task unchanged.

| Diagnostic | Means |
|---|---|
| `clarify_unknown_question` | An answer names a question that is not in the set. |
| `clarify_invalid_answer` | A `YesNo` answer is not `y` or `n`. The app sends these from a radio group, so anything else is a client defect. |
| `clarify_no_answers` | No answer was given to a question that applies. Defer means "not now"; Reject means "not these". |
| `clarify_description_too_large` | The description would be over 16,384 bytes. |
| `advisory_target_inactive` | The Task is no longer active. |
| `advisory_not_a_clarification` | The candidate is not a clarification proposal. Use Admit. |
| `advisory_answer_required` | From Admit, not from Answer: the candidate is a clarification proposal. Answer it. |

A stale `observed_version` is HTTP 409, as for every review action. So is an
answer to a candidate that is deferred, rejected or already admitted: a
deferred question set is resurfaced before it is answered.

## Rounds

`round` is one more than the number of clarification candidates admitted for
the Task. A question set that was proposed and rejected, or is still waiting,
is not a round.

From P1B-51 the run response carries it: `round` is present on every Clarify
run that selected a Task, including one refused with `clarify_already_queued`,
and absent for SuggestTags and for `clarify_no_task`. It is how a caller tells
the two meanings of `clarify_no_questions` apart; the diagnostic's message is
the same on every round.

## Limits

1. **One Task per run**, and one open interview per Task.
2. **Automatic selection is round one only.**
3. **No round cap.**
4. **No history of other Tasks in the prompt.**
5. **No tags from the interview.**
6. **The description has no structure.** It is text. An operator who edits
   it by hand changes what the next round reads.
7. **Whether a later round repeats a question is the model's doing.** The
   prompt tells it not to. That is what `advisory.model` exists to change.
