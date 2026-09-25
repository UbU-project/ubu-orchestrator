# P1B-31 verification

## Scope and results

Only `ubu-orchestrator` changed, from clean HEAD
`e139182829cec775b14f410fcfbd130ca1615897`, on branch
`p1b-31-calendar-reconcile`. All 11 sibling repositories retained their original
HEADs and clean statuses. The ticket's pinned read-only repositories match the
stated baseline. The specified Calendar and GitHub reconciliation implementation
was read before edits, along with UBU-D0254 and UBU-D0257 in the pinned design repo.
No credentials or real Quick UbU data were read or used.

**252 tests passed, zero failed or ignored**, from 239: exactly six new pure unit
tests and seven new HTTP integration tests. Count = sum of Cargo's passed counts
across unit, integration and doc-test summaries; zero-test targets contribute zero.
The complete suite passed at each code section: A 245, B 245, C 245; documentation
section D retained C's green code state. Section E passed 252. Normal compiler and
test concurrency was retained; no memory/compiler runtime caps were added.

Clippy used the same checkout, toolchain and command before and after in this run:

```sh
cargo clippy --locked --offline --all-targets --message-format=json
```

Counting method: collect JSON `compiler-message` records with level `warning` for
package `ubu_orchestrator`; deduplicate by `(code, message, primary span file,
line, column)`. This counts repeated lib/lib-test emissions once, excludes dependency
warnings and excludes Cargo summary text. **10 before, 10 after; delta 0.** The
warning identities are identical, not merely the counts. No lint allowances added.

**Cargo.lock is byte-identical**, checked directly against a pre-edit copy.
SHA-256 before and after:

```text
4a73890bbac0991eaad68d129e5131e945336c28cf806b859369dbbf963879f9
```

Cargo.toml is unchanged. No dependency, package version, Git pin or sibling HEAD
moved. No migration, new repository or clone was needed. OpenAPI was regenerated
with `cargo run --locked --offline --example generate_openapi`, including both
Calendar routes and Calendar-specific request/response/conflict schemas.
`git diff --check` and individual `rustfmt --check --edition 2021` passed. Only the
four new Rust files were formatted; no whole-repo formatting ran. No earlier
source/test diagnostic code string was changed or removed, and no existing test
needed modification.

## Offline and privacy evidence

No test performed a network call. All suites used a sanitized environment
(`env -i` retaining only PATH, HOME and CARGO_NET_OFFLINE=true), Cargo's
`--locked --offline` flags, and an inherited seccomp filter rejecting socket domains
AF_INET and AF_INET6. The final entire Cargo process tree was also traced:

```sh
env -i PATH="$PATH" HOME="$HOME" CARGO_NET_OFFLINE=true \
  python3 /tmp/p1b-31-results/offline-test.py \
  strace -f -qq -e trace=network -o /tmp/p1b-31-results/E-network.log \
  cargo test --locked --offline -- --nocapture
```

The trace has **zero AF_INET/AF_INET6 attempts and zero connect() calls**. Its only
socket family is AF_UNIX for local build-tool IPC. The temporary wrapper loads
libseccomp, installs EPERM socket-family rules and execs the command. Ptrace
required sandbox escalation; the network restriction remained active. No runtime
configuration file was changed. Baseline snapshots, per-section test logs, clippy
JSON output and the final trace are in `/tmp/p1b-31-results` on the working machine.

The new endpoint tests use Axum Router::oneshot, in-memory SQLite and only
RecordingCalendarApi. Wipes and edits call that recorder directly before an HTTP
reconcile; no request can supply an observation. Live refusal tests stop in the
unchanged ensure_available before client construction, list calls or persistence.
No Google/OAuth/browser transport or manual live smoke was exercised.

Every fixture is synthetic, including the Task IDs minted by the capture endpoint.
No real calendar ID, credential, token, client ID or client secret appears in the
diff, logs, responses or newly stored test data. All changed/new files were reviewed
and scanned for Google token/client-secret/client-ID/calendar-ID patterns, with
zero hits. Existing Google errors remain sanitized by P1B-30; this ticket does not
alter credential handling or log raw provider errors. Only observation events and
projection bookkeeping are persisted, with no canonical admission of external data.

## Required verbatim test evidence

Test 2: conflict list after applying two events and wiping the recording calendar:

```json
[{"conflict_type":"missing","external_id":"01a0d911b94476208f5426337c702bff","message":"UbU applied this event and the calendar no longer has it","summary":"Breakfast"},{"conflict_type":"missing","external_id":"01a0d911b9497a81b796090440899ded","message":"UbU applied this event and the calendar no longer has it","summary":"Standup"}]
```

Reconcile alone leaves the applied belief unchanged. Explicit repair reports two
dropped events, zero updated events and an empty applied set. The next preview's
operations, verbatim:

```json
[{"event":{"color_id":"3","end_at":"2026-09-25T09:30:00Z","external_id":"01a0d911b94476208f5426337c702bff","reminders_minutes":[],"start_at":"2026-09-25T09:00:00Z","summary":"Breakfast","task_id":"task_01a0d911b94476208f5426337c702bff","transparent":false},"kind":"create"},{"event":{"color_id":"3","end_at":"2026-09-25T10:30:00Z","external_id":"01a0d911b9497a81b796090440899ded","reminders_minutes":[],"start_at":"2026-09-25T10:00:00Z","summary":"Standup","task_id":"task_01a0d911b9497a81b796090440899ded","transparent":false},"kind":"create"}]
```

The test also applies that ordinary preview and reconciles again to matched.
Repair itself makes no external write; its only preceding client call was list.

Test 3: after dragging Breakfast in the recorder, repairing the stored observation
and previewing again, exactly one update, with no create/delete:

```json
[{"event":{"color_id":"3","end_at":"2026-09-25T09:30:00Z","external_id":"01a0d911b9497a81b79608da8da46c50","reminders_minutes":[],"start_at":"2026-09-25T09:00:00Z","summary":"Breakfast","task_id":"task_01a0d911b9497a81b79608da8da46c50","transparent":false},"kind":"update"}]
```

Test 4: the foreign Dentist remains unchanged in the three-event recorder, is
excluded from the repaired two-event applied set, remains in remaining_conflicts,
and appears in no operation. Verbatim evidence:

```text
repaired applied-set size: 2; observed size: 3; operations: []
```

Canonical objects, versions/statuses, Logs, Plans and Calendars are unchanged by
both reconcile and repair. The recorder shows only ListEvents, with no foreign
patch/delete/adoption. A foreign-only conflict returns observed rather than drifted.

Test 5: with no applied history, an observed ID deriving from a newly captured,
active **unscheduled** Task is classified verbatim as:

```json
[{"conflict_type":"unrecorded","external_id":"01a0d911b9647492b07ac4eee4fbfdae","message":"this event matches a known Task but UbU has no applied record; it will not be adopted","summary":"Unscheduled synthetic Task"}]
```

Repair does not adopt that event: applied_event_count is zero and the unrecorded
conflict remains. After that Task is explicitly completed, a fresh reconciliation
classifies the still-unowned event as foreign, proving that known-ID evidence uses
active Tasks rather than stale Calendar steps.

## Further checks within the seven endpoint tests

- Unchanged apply/reconcile yields matched with no conflicts or diagnostics; the
  recorded observation and original applied set match the persisted payload.
- Missing/unknown request schema and an attempted observed_events request field
  are refused without client calls.
- An injected SQLite failure on the repaired-marker update rolls back the result
  insert as well. Removing the failure permits repair normally.
- Two concurrent HTTP repair requests produce one success and one 409. Further
  repair returns `calendar_reconciliation_already_repaired`; only one result was
  added. Missing IDs return 404; a GitHub-shaped record returns 400 and adds no
  Calendar result. This tests the atomic one-shot marker rather than only a flag.
- Live without paths returns 503 `calendar_live_export_unconfigured`; configured
  but disabled Live returns 403 `calendar_live_export_not_enabled`. Both leave
  recorder calls, reconciliations, results and Logs empty.

The six pure tests additionally exercise every DesiredEvent field's drift behavior,
conflict sorting, input-order independence, ownership without active-Task evidence,
foreign IDs sharing the Task alphabet, no adoption, empty input and the normal diff
producing creates/updates from the repaired belief.

## Judgment calls

No disagreement with any of the eight requested implementation choices:

| Call | Implementation and assessment |
|---|---|
| 1. Applied-record ownership | Agreed. Only recorded applied IDs are owned; no ID-shape or title heuristic. |
| 2. Unrecorded evidence | Agreed. All derivable active-Task IDs distinguish unrecorded from foreign without conferring ownership. |
| 3. Explicit repair | Agreed. Reconcile persists a report only; repairing requires the second endpoint. |
| 4. Belief-only repair | Agreed. Retain only previously owned observed IDs at observed values; neither foreign nor unrecorded events enter the applied set. No Google writes. |
| 5. No export gate for read/local repair | Agreed. Neither path invokes Legitimizer or emits boundary adjudication Logs; the subsequent ordinary apply still does. |
| 6. Live availability checks | Agreed. The unchanged P1B-30 ensure_available runs before a live read. |
| 7. Observe the recorder | Agreed. Mock list_events observes the selected stateful recording client; no observed-events request field exists. |
| 8. Foreign reporting only | Agreed. No ExternalEvent, Task, planning constraint, Device authority or capture namespace is created. Quick UbU import_from_calendar remains a separate-feature precedent. |

## Ambiguities and literal choices

- Section A remains a pure module. Section B's database helper and C's orchestration
  live in a separate service module so classification/repair acquire no I/O.
- The repair request body was unspecified: it takes only the path ID and no body,
  mode, authority or credentials. The stored schema is checked before loading a
  Calendar observation, preventing cross-surface repair.
- Reconcile uses schema `ubu.orchestrator.calendar_reconciliation.v1`; repair
  responds with `ubu.orchestrator.calendar_repair.v1`. Both use typed Calendar
  conflict objects, never GitHub's label-shaped conflict DTO.
- Reconcile captures both applied and observed snapshots. Repair uses those exact
  snapshots, not a refreshed list or a newly queried ownership set. The process
  mutex serializes reconcile/repair with preview/apply, but no freshness rejection
  is added between calls: stale/partial trust is a known limit, explicitly left
  unfixed. The endpoint's one-shot marker and result are one SQL transaction.
- A local repair result has ordinary Calendar result schema/status applied, empty
  operation_results, and explicit repair/reconciliation_id markers. This means
  the repaired belief was stored, not that Google was written. An empty repaired
  snapshot supersedes earlier applied snapshots through existing row-order lookup.
- No prior result is a valid empty ownership base (needed for the reset case).
  The payload records result_id null; the existing NOT NULL SQL metadata uses an
  empty result-id sentinel and a reserved typed ProjectionPreview ID. No preview
  row or prior delivery is invented. The reserved ID also preserves the projection
  result's existing preview-id type when an empty observation is repaired.
- The status column marks repaired after commit while payload status retains the
  original observation status. The payload additionally records repaired=true,
  repaired_result_id and repaired_at, preserving what the operator reviewed.
- Mock without an injected recorder follows approve's existing behavior: seed a
  recorder from the applied set. Stateful wipe/edit tests inject the same recorder
  used by apply. No new dependency or changes to CalendarApi are needed.
- Classification covers events returned by CalendarApi. P1B-30's live reader still
  skips entries its DesiredEvent wire model cannot represent (such as all-day or
  calendar-default-reminder events) and reports diagnostics. Those parser limits
  are not changed here; they can produce the partial-list risk explicitly retained
  below. The operator guide calls out reviewing such diagnostics before repair.
- Existing mock/live/single-calendar applied-set scoping is unchanged. Use the
  scratch calendar's own database, as documented in CALENDAR_LIVE.md.

## Known limits (verbatim from the ticket)

1. **Reconciliation is on demand.** Nothing polls. UbU notices a wiped or edited calendar only when the operator asks, so the phone can disagree with the desktop until then.
2. **Drift detection is whole-event.** Any difference between the observed and applied event is one `drifted` conflict; the response does not say which field moved.
3. **Foreign events are not planning constraints.** A real meeting on the calendar is reported and ignored. The planner still does not know the operator is busy then.
4. **Repair trusts the observation completely.** If a read returns a partial or stale list, repair will drop events that do exist, and the next apply will recreate them. Recreation is safe because the ids are derived, but the calendar will churn.
5. **No history.** Each reconciliation is stored, but there is no view of how a calendar drifted over time.


## Section commits

One signed commit per lettered section, with no force-push:

```text
4d0372b P1B-31 A: classify Calendar drift and repair the applied belief
2db8dbd P1B-31 B: derive reconciliation evidence from active Task ids
6a6b07a P1B-31 C: expose Calendar observation and atomic belief repair
b630849 P1B-31 D: document Calendar reconciliation and wipe-and-copy repair
```

The fifth commit, section E, contains the seven HTTP integration tests and this
verification report. The wipe-and-copy operator procedure is documented in
[CALENDAR_RECONCILE.md](CALENDAR_RECONCILE.md); no real Calendar was accessed or
modified during this ticket.
