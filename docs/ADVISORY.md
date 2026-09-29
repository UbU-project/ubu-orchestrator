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
Revert uses `DELETE /setting/{name}`. There are no built-in advisory defaults.
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
excluded. The response names every selected ID/title, every created candidate
ID, the enqueued count, the validated returned proposals in `report`, and any
diagnostics. A Task may be omitted by the model; a successful empty proposal
array creates nothing. Repeated identical queued proposals are not duplicated.

An absent Setting returns HTTP 200 with `advisory_unconfigured` naming it and
performs no selection or model call. Connection, timeout, HTTP status, size and
malformed-result failures return failed-result diagnostics with no candidates,
not HTTP 500. Invalid request version, producer or limit is a request rejection.
No raw HTTP error body is returned or logged by the transport.

## Exactly what is sent

**Task IDs and titles are the only Task data sent to the model.** The minimized
submission payload is an array, for example this synthetic fixture:

```json
[{"id":"task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e70","title":"Synthetic lunar teapot 0"}]
```

This array becomes the JSON string in Ollama's `prompt`. The HTTP body also
contains the configured `model` name, `stream: false`, fixed system instructions
and a fixed JSON output schema in `format`. It never serializes the submission
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

The submission permits only Tag proposals and diagnostics. Timeout is 120 seconds
and result size is bounded at 256 KiB, both while receiving bytes and after
expanding candidate metadata. Declared `ComputeBudget` is 120,000 CPU milliseconds
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

`Advise`, `Clarify` and `Batch` are not implemented. Advise would propose richer
advice; Clarify would turn uncertain intent into explicit clarification proposals;
Batch would provide unattended scheduling/batching. Each needs its own selection,
authority and disclosure decision. SetModel is configuration, not another screen.

Runs remain manual, payloads remain IDs/titles only, endpoints remain loopback,
and there is no model installation or management. Rejection suppresses exact
candidates, not patterns. Uncaptured recurring commitments still occupy no
planning capacity; see the temporary **Busy** blocking-event workaround in
[Calendar bootstrap](CALENDAR_BOOTSTRAP.md). Calendar export still records no
approver, and the Routines screen still cannot clear an override.
