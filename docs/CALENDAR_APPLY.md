# Applying Calendar projections offline

P1B-28 established the event mapping and deterministic diff. P1B-29 adds the
recording client, persisted previews and applied sets, and gated apply. P1B-30
now adds the real Google transport and session enablement; see
[CALENDAR_LIVE.md](CALENDAR_LIVE.md) for configuration and the scratch-calendar
procedure. External-edit reconciliation follows in P1B-31. The adapter lives in `ubu-orchestrator`; the
`CalendarApi` trait is the extraction seam if an independent release is needed.

## Existing contracts and the two additions

Core `ProjectionOperation` already targets a surface-agnostic `SourceRef` and
carries an optional JSON payload. `Legitimizer::gate_export_projection` evaluates
authority and resolved policy without inspecting the surface. Store preview and
result records already accept free-form payloads. Those contracts are reused.
Only two contract additions were needed: operation kind `delete`, and the optional
unconstrained `payload` property missing from the operation JSON schema despite
already being serialized by core. No dependency or new repository was added.

Calendar operations target `{source_kind: "google_calendar", source_id: <event
id>}`. P1B-28's reversible event id mapping strips `task_` and validates the tail
against lowercase `a-v` and `0-9`, length 5–1024. Create and update payloads carry
the desired event fields; delete carries only `external_id` and `summary`.

## Preview and approve

`GET /projection/calendar/preview` still describes `/calendar/current`, carrying
its Plan id and stale flag. It now also returns `preview_id` and persists a
projection preview containing desired events, operations, the applied-set base,
and resolved policy. It changes projection records only, not canonical Tasks,
Plans or Calendar data. Repeated previews have identical semantic content while
each gets a new preview id. The request makes no Calendar client calls.

The optional query parameter `no_external_export=true` resolves a denying policy,
mirroring the existing GitHub preview's policy input. Otherwise the current
controller policy summary explicitly permits automation-worker export, subject
to the gate. The stored summary is the policy passed to every apply adjudication.

Approve the stored preview with `POST /projection/calendar/approve`:

```json
{"schema_version":"ubu.orchestrator.calendar_projection_approval.v1","preview_id":"<preview id>","authority_source":"automation_worker","export_mode":"mock"}
```

All four fields are required. Missing or unknown schema versions get named HTTP
400 diagnostics; missing required typed fields use the ordinary request
extraction error. A valid processed batch returns HTTP 200, with schema
`ubu.orchestrator.calendar_projection_result.v1`, preview id, aggregate status,
applied events, per-operation outcomes, and diagnostics. Aggregate status is
`applied`, `partial`, or `failed`; operation status is `applied`, `skipped`, or
`failed`. A gate-denied batch is represented by a failed result with skipped
operations and rejection diagnostics, not by a successful delivery claim.

`export_mode: "live"` requires configured credential paths and per-process
enablement. Missing paths return HTTP 503 `calendar_live_export_unconfigured`;
a disabled session returns HTTP 403 `calendar_live_export_not_enabled`, before
any client call or delivery record. Live never substitutes the recorder.
Mock mode uses `RecordingCalendarApi`, seeded from the persisted applied set.
Tests can inject a recorder through `AppState::with_calendar_api` to inspect its
ordered calls and simulate operation failures.

## Deny-by-default boundary

Every operation, including deletes, is lowered to core `ProjectionOperation` and
passed to the existing `Legitimizer`. The caller's authority is preserved; it is
not silently replaced by worker authority. Only `automation_worker` can receive
an export permit, and only when the resolved policy is accepted and explicitly
has `local_only: false` and `no_external_export: false`.

User-equivalent authority, other authorities, unresolved policy, disallowing
policy, or policy needing review receive no permit. Neither a title nor a surface
name grants permission. This follows UBU-D0254 and UBU-D0255's controller-owned
admission boundary; per UBU-D0257, Google Calendar is a projection surface, not a
Device or mutation authority. UBU-D0275's desktop-to-Calendar switch configuration
does not change that boundary.

The controller appends `compartment_boundary_decided` through an ordinary mutation
envelope for every adjudication, before attempting delivery. The Log records the
actor, requested authority, policy decision, reason, and `google_calendar`
provenance. Dispatch requires a matching core permit. A rejection produces
`calendar_export_rejected` and no client call for that operation. A permitted
client failure produces `calendar_operation_failed` in the result.

## What the applied set means

Preview's existing side is exclusively the latest Calendar applied-set snapshot
in `projection_results`. GitHub results are excluded by schema version. The
client's `list_events` method exists at the boundary but preview and apply never
call it: external observations and external-edit conflicts belong to
reconciliation, not this diff.

A fully successful apply records `applied`; a mixed batch records `partial` and
includes only the effects of operations that actually succeeded. The snapshot
starts with the previous applied set, replaces successfully created/updated
events, removes successfully deleted events, and retains old events when an
update/delete fails. Entirely failed batches do not replace the successful
applied-set history. Partial snapshots do replace it, so the next preview does
not duplicate operations that already landed.

The latest snapshot is selected by durable insertion order, including when a
fixed or adjusted clock gives multiple results the same timestamp. A successful
empty apply still records the current set. An empty applied set after deleting
all events is also a real snapshot, not a signal to fall back to an older set.

Operations are attempted once, independently, in deterministic create/update/
delete order, each group sorted by external id. There is no automatic retry or
backoff, except P1B-30's single insert-conflict-to-patch transition. A later preview proposes failed work again. Successful apply followed by
an unchanged preview produces zero operations.

Preview and apply share a single-Device mutex. Apply verifies its stored base
against the latest applied set; a changed base returns HTTP 409
`calendar_projection_conflict` and asks for a new preview. This prevents an old
preview from replaying operations against a different applied set. It does not
compare that set with Google or imply any external reconciliation.

## Known limits

1. **Transport is covered manually.** P1B-30 adds Live; offline tests exercise the wire model and recorder. Only the scratch-calendar procedure exercises Google.
2. **The applied set is UbU's belief, not the calendar's state.** If someone edits or deletes an event in Google, UbU will not notice and the next preview will propose nothing. Reconciliation is P1B-31.
3. **No retry and no backoff.** A failed operation is recorded as failed and re-proposed by the next preview. There is no queue and no automatic second attempt.
4. **One calendar.** P1B-30 selects it with `UBU_GOOGLE_CALENDAR_ID`; routing by category or compartment is not supported.
5. **The boundary Log records adjudication, not delivery.** A permitted operation that then fails in the client is visible in the result payload, not in the boundary Log entry, which was written when the gate decided.
