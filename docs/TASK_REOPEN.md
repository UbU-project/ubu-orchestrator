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

## Effects are not reversed

A Task's effects are applied to `UniverseState` when it completes. Reopening
does not reverse them. The Calendar undo never has, and says nothing.

This path says so. When the Task carries effects, the response has:

```json
{"code":"reopen_effects_not_reversed","message":"The Task is active again. The effects it applied when it completed were not reversed, and will be applied again if it is completed again"}
```

Reversing effects is not attempted. What a reversal should mean, when
something else has changed the same fact since, is not decided.

The Calendar path is unchanged.
