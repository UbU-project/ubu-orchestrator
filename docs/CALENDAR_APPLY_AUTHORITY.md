# Calendar apply authority

From P1B-43, the Calendar export is gated as the automation worker, whoever
approved it. This is what the GitHub export has always done.

## The two paths, and how they disagreed

The GitHub path, `src/services/projection_service.rs`, has approved
successfully since Phase 1. It ignores the request's `authority_source` at the
gate:

```rust
let adjudication = gate_export_operation(
    core_operation,
    operation,
    Some(&policy_summary),
    AuthoritySource::AutomationWorker,
    state.actor_identity_id(),
);
```

The Calendar path, `src/services/calendar_apply.rs`, passed the request's
authority straight through, and then required the permit to be the worker's:

```rust
let gate = gate_export_operation(state, &core, stored.policy_summary.as_ref(), authority);
```

```rust
if permit.operation_id() != core.operation_id
    || permit.authority_source() != ubu_core::AuthoritySource::AutomationWorker
{
    return Err(AppError::Internal(
        "Calendar export permit does not match the operation".into(),
    ));
}
```

The core gate gives a permit to `automation_worker` only. So a Calendar approve
carrying any other authority got no permit: every operation was skipped with
`calendar_export_rejected`, nothing reached the Calendar, and the result was
`failed`.

Every approve in `ubu-ui` sends `authority_source: "user"`, on both paths. The
GitHub approve worked. The Calendar approve could not.

## What the code does now

```rust
let gate = gate_export_operation(
    state,
    &core,
    stored.policy_summary.as_ref(),
    ubu_core::AuthoritySource::AutomationWorker,
);
```

One call site changed. The permit check is unchanged in substance and now
lives in `calendar_apply::ensure_permit_matches`, so that a test can reach it.
It holds by construction on the normal path and is kept as the invariant
between the gate and dispatch.

- **The export is performed by the automation worker in both paths.** The
  operator approves; the worker writes.
- **The request's `authority_source` describes the approver.** It is still a
  required field of `POST /projection/calendar/approve`, and every value it
  accepted before is still accepted. It is not used at the gate. In
  `calendar_apply::approve` the parameter is now unused and is named
  `_approver_authority`.
- **The boundary Log records `automation_worker`**, in the payload and in its
  provenance, as the GitHub path's does.

Nothing else about the export changed: not the gate, not the policy summary,
not `no_external_export`, not live-mode enablement, and not what is applied.

## What this unblocked

P1B-41's acceptance step 3 is to approve a Calendar preview in the application
and confirm the events appear on the dummy calendar. With the application
sending `user`, that step could not pass. P1B-42's step 5 needs a calendar UbU
has applied to, and could not pass either. P1B-42 recorded the blocker in
`CALENDAR_BOOTSTRAP.md` and left it alone.

## What this reverses

P1B-29's judgment call 7 required `automation_worker` from the caller, and its
test 4 asserted that a `user` approve was rejected with no client call.
`CALENDAR_APPLY.md` said the caller's authority "is not silently replaced by
worker authority". P1B-43 replaces it, deliberately. That test now asserts the
opposite for every authority value the API accepts.

## Limits

1. **The request's `authority_source` is accepted and not used at the gate**,
   in both projection paths.
2. **The approver is not recorded.** The boundary Log records the worker. No
   Log entry or stored result says which authority approved a Calendar export.
   The GitHub path stores the approver in its approval record; the Calendar
   path has no approval record.
3. **A Calendar operation result carries no authority.** The GitHub result
   body has an `authority_source` on each operation. The Calendar result body
   has `operation_id`, `status` and `message` only. For the Calendar, the
   exporter's authority is in the boundary Log.
4. **The authority check in the core gate no longer distinguishes callers**
   on either path. What still refuses an export is the resolved policy, the
   conflict check against the applied set, and live-mode enablement.
