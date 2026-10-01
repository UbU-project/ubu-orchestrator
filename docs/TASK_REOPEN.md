# Undo of a completion

`POST /task/{task_id}/reopen` returns a completed Task to `active`. It is for
a completion made in the app by mistake.

Until P1B-48 the only path from `completed` back to `active` was the Calendar
one, `reopen_calendar_completion`, which fires when the operator removes the
colour from an event in Google Calendar. A Task completed in the app by a
misclick could not be fixed in the app.

## The request

```json
{"schema_version":"ubu.orchestrator.task_action.v1","completion_log_id":"log_…"}
```

`completion_log_id` is the `log_id` the completion returned.

**The completion to undo is named.** It must be the Task's latest completion.
Undo of "whatever the last thing was" is how the wrong thing gets undone.

## The response

```json
{"schema_version":"ubu.orchestrator.task_action.v1","log_id":"log_…","task_id":"task_…","completion_log_id":"log_…","task_status":"active","diagnostics":[]}
```

`log_id` is the reopen decision that was recorded.

## What it records

The same as the Calendar undo, under the same `task_action_lock`: the Task's
status becomes `active`, and a `decision_recorded` Log entry is appended with
`action: "reopen"` and `decision: "task_reopened"`, whose `object_refs` are
the Task and the completion.

It carries **no `source` marker**. This undo did not come from Google.

## Refusals

All are HTTP 409 and change nothing.

| Diagnostic | Means |
|---|---|
| `reopen_not_completed` | The Task is not completed. This is what a second undo gets. |
| `reopen_no_completion` | The Task is completed and no completion is recorded for it. |
| `reopen_stale_completion` | The named id is not the Task's latest completion. |

A missing or unknown `schema_version` is HTTP 400, with the codes every Task
action uses. An unknown Task is HTTP 404.

## There is no placement restriction

The Calendar undo refuses a Static Task and a Task captured from the
calendar, because removing a colour only means "not done" for an event UbU
projected as Dynamic. In the app the operator is undoing their own click,
whatever the Task is.

## Effects are not reversed, and are applied once

A Task's effects are applied to `UniverseState` when it completes. Reopening
does not reverse them. The Calendar undo never has, and says nothing.

This path says so. When the Task carries effects, the response has:

```json
{"code":"reopen_effects_not_reversed","message":"The Task is active again. The effects it applied when it completed were not reversed, and will not be applied a second time if it is completed again"}
```

**From P1B-53 effects apply once per Task.** Until then, complete, undo,
complete applied a Task's mutations twice: an `increment_numeric` counted the
same piece of work two times. The operator decided that effects are
idempotent per Task.

When a Task completes, the orchestrator asks the decision log whether that
Task already has a completion. The question is asked before this completion's
own decision is written, so a completion found there is an earlier one. If
there is one, the Task still completes, its effects are **not applied**, and
the response has:

```json
{"code":"task_effects_already_applied","message":"Task `task_…` completed before, so its recorded effects were not applied a second time"}
```

The two diagnostics say the same thing from either side of the undo. There is
no new column and no flag on the Task: the log is the record.

What follows from keying on "has completed before":

- it is once per Task, not once ever. A routine's occurrences are separate
  Tasks, so each night's occurrence applies its effects on its own first
  completion;
- a Task with no `effects`, or with no mutations listed, behaves as it always
  did and reports nothing;
- effects added to a Task after its first completion, while it is reopened,
  are never applied. That is the cost of not recording which effects were
  applied; it has not come up.

Reversing effects is not attempted. What a reversal should mean, when
something else has changed the same fact since, is not decided.

**A stated future direction, not built.** The operator's note, as the P1B-53
ticket records it: a future version may ask the operator whether to reverse
`UniverseState` during an undo. That is a decision of its own and wants
recording in `ubu-design`. Nothing here implements it, and no code carries a
TODO for it.

The Calendar path is unchanged.
