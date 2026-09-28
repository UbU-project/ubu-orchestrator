# P1B-44 verification

## Result and acceptance boundary

Recurring instances now survive timed-event parsing, reconcile as foreign, and
are deliberately refused at capture without Task admission or ownership writes.
The Calendar screen groups those refusals separately and warns about the capacity
gap. Apply, repair, colour partition, export authority and planning behavior are
unchanged. No route, model field, schema version or client method was added.

All automated checks use synthetic data. **Live dummy-account acceptance remains
for the operator**; the six steps are in [Calendar bootstrap](CALENDAR_BOOTSTRAP.md#recurring-instances-observe-name-refuse).
P1B-41, P1B-42 and P1B-43 are accepted; their former approval blocker is resolved.

## Revisions and landing order

Baselines: orchestrator `5ea1d85`, UI `c94b6ac`, devshell `324e12a`.
The three changed repositories use `p1b-44-foreign-tolerance`; read-only sibling
repositories retain their original branches and revisions. All started clean
after the operator-authorized local exclude exception for acceptance artifacts.
No artifact names or contents were included in a commit; the excludes exist only
in local Git metadata.

The backend implementation was pushed first at `c5168f38d7e800d7009c8685a5897c1deb4862a9` (sections A–C).
Final repository tips land in this order:

| Order | Repository | Final revision |
| --- | --- | --- |
| 1 | ubu-orchestrator | The section F commit containing this report, on `p1b-44-foreign-tolerance` |
| 2 | ubu-ui | `2796a069607f4fce62a7f632f4e9da12ac7ddc24` (section E) |
| 3 | ubu-devshell | The final pin commit on `p1b-44-foreign-tolerance` |

Concrete final SHAs and the unnormalized inventory are also recorded in
[`ubu-devshell/docs/P1B-44_PINS.md`](https://github.com/UbU-project/ubu-devshell/blob/p1b-44-foreign-tolerance/docs/P1B-44_PINS.md)
and the delivery response. A document cannot contain its own commit's SHA or the
SHA of a later commit that pins it without introducing a cycle.

One commit per lettered section: A, B, C and F in orchestrator; D and E in UI.
The ticket specifies a final devshell pin bump but does not letter it; it is one
additional pin commit. UI and devshell commits have `Co-Authored-By`; orchestrator
commits do not. Pushes are ordinary branch pushes, with no force-push or merge.

**No dependency pin moved.** Only the two release inventory entries in
`ubu-devshell/pinned-revs.toml` move, last, as required. No manifest or lockfile
changed. Read-only baselines remain core `c77c0a2`, store `7b24cd8`, schemas
`4974166`, planning kernel `84b6d0d`, GitHub adapter `4c7e3b6`, quick-ubu `9ccc8b8`,
design `f7c4a1d`, and brand `faf2005`.

## Checks

| Check | Before | After |
| --- | --- | --- |
| Orchestrator `cargo test --locked --offline` | 353 passed | 360 passed |
| UI `npm test -- --reporter=verbose` | 43 passed | 47 passed |
| Clippy, all targets: warning messages / unique warnings | 16 / 9 | 16 / 9 |
| `scripts/check-ui-contract.sh` live path constants | 33 / 33 | 33 / 33 |

Clippy command: `cargo clippy --locked --offline --all-targets --message-format=json`.
Count only JSON `compiler-message` records whose message level is `warning`.
Deduplicate by warning message, diagnostic code, and primary span
(file, start line, start column). This removes repeated library/test-target
emissions. **Delta: zero raw and zero unique warnings; no new unique warning.**
Existing warnings were not fixed outside this ticket.

`npx --no-install tsc --noEmit` and `npm run build` both pass. Devshell adds no
unit tests; its required before/after integration contract check passes, including
seven loopback requests and all 33 path constants. The release pin check is
repeated after the final bump. No path constant, route, generated API schema or
schema-version string changed.

All three locks were compared byte-for-byte against saved baseline bytes and
remain identical: orchestrator `Cargo.lock`, UI `package-lock.json`, and UI
`src-tauri/Cargo.lock`. There are no new dependencies.

Section A updated **two existing test expectations** in `tests/calendar_wire.rs`:
the invalid-ID fixture now uses a non-string ID rather than a readable unusual
ID, and the malformed timestamp diagnostic now includes the readable ID. Test
counts are unchanged by those edits. The seven new tests live in
`tests/calendar_foreign_tolerance.rs`; the four new UI tests (44–47) extend
`tests/calendar.test.tsx` using the mocked Tauri HTTP plugin. Section A's eight
wire tests and section B's 17 capture/reconciliation tests passed before their
commits; the complete 360-test suite passed before section C. Section D passed
all 43 existing UI tests and TypeScript before its commit; section E passed 47,
TypeScript and the production build.

## Verbatim synthetic evidence

These lines are emitted by the seven-test regression target with `--nocapture`.
Test 1 shows the parsed `DesiredEvent`. Test 2 is the first reconciliation.
Test 3 includes the exact refusal and object counts. Test 4 compares the first
and second reconciliation, checks no capture ownership write, and runs a mock
preview/apply without duplicating the meeting. Test 6 identifies malformed
entries by index and ID when readable; no summary or bad timestamp content is
quoted by the diagnostics.

```text
P1B44_TEST6={"code":"calendar_event_skipped","message":"list event `aaaaa` entry 0: missing or invalid summary"}
P1B44_TEST6={"code":"calendar_event_skipped","message":"list event `bbbbb` entry 1: invalid start.dateTime"}
P1B44_TEST1={"external_id":"abc123def456ghij_20260928T163000Z","task_id":"task_abc123def456ghij_20260928T163000Z","summary":"Synthetic lunar teapot rehearsal","start_at":"2026-09-28T16:30:00Z","end_at":"2026-09-28T17:00:00Z","color_id":"3","transparent":false,"reminders_minutes":[10]}
P1B44_TEST6={"code":"calendar_event_skipped","message":"list event `abc123def456ghij_20260928T163000Z` entry 2: cancelled event"}
P1B44_TEST6={"code":"calendar_event_skipped","message":"list event `*` entry 3: missing or invalid id"}
P1B44_TEST2=[{"conflict_type":"foreign","external_id":"abc123def456ghij_20260928T163000Z","message":"Calendar event `abc123def456ghij_20260928T163000Z` cannot be captured: its id cannot be a UbU Task handle, so UbU cannot own it","summary":"Synthetic lunar teapot rehearsal"}]
P1B44_TEST3={"diagnostics":[{"code":"capture_event_not_ownable","message":"Calendar event `abc123def456ghij_20260928T163000Z` cannot be captured: its id cannot be a UbU Task handle, so UbU cannot own it"}],"objects_after":0,"objects_before":0}
P1B44_TEST4={"applied_records_after_capture":0,"applied_records_before":0,"events_after_apply":1,"first":[{"conflict_type":"foreign","external_id":"abc123def456ghij_20260928T163000Z","message":"Calendar event `abc123def456ghij_20260928T163000Z` cannot be captured: its id cannot be a UbU Task handle, so UbU cannot own it","summary":"Synthetic lunar teapot rehearsal"}],"second":[{"conflict_type":"foreign","external_id":"abc123def456ghij_20260928T163000Z","message":"Calendar event `abc123def456ghij_20260928T163000Z` cannot be captured: its id cannot be a UbU Task handle, so UbU cannot own it","summary":"Synthetic lunar teapot rehearsal"}]}
```

Test 2 also lists an invented instance outside the horizon and asserts it is
absent from conflicts and the refusal diagnostics. Test 5 checks ordinary capture
with mapped, unmapped and absent colours and preserves the ambiguous-colour
message. Test 7 uses an all-day recurring instance and still gets
`capture_all_day_unsupported`, no Task and no applied record.

UI tests verify separate groups and backend reasons, absence of controls within
both foreign groups, a two-event horizon count that disappears on a subsequent
zero-count reconciliation, singular wording, and byte-for-byte matching refusal
words between reconcile and capture. No rendered test calls global fetch.

## Five downstream claims, verified

1. **Reconcile:** [`calendar_reconcile::classify`](../src/services/calendar_reconcile.rs)
   builds maps keyed by exact `external_id`. Only the applied record confers
   ownership. [`known_external_ids`](../src/services/calendar_reconciliation_service.rs)
   derives Task evidence through `external_id`, so an unownable instance ID is
   neither an applied owner nor a known/unrecorded Task ID in the supported
   flow. Tests 2 and 4 prove foreign classification before and after refusal.
2. **Capture:** [`capture`](../src/services/calendar_capture.rs) filters conflicts
   to `foreign`, then `plan_capture` applies the existing `external_id_for`
   guard through `not_ownable_diagnostic`. The event never enters admission or
   the loop that appends to applied ownership. With no accepted gesture or Task,
   `recorded` remains false and no projection result is written. Tests 3 and 4
   assert unchanged object/admission counts and unchanged applied-record count.
3. **Apply/projection, corrected with operator approval:**
   [`preview`](../src/services/calendar_apply.rs) first builds desired events from
   Plan steps via [`desired_events`](../src/services/calendar_projection.rs),
   preserving source metadata for already-captured Tasks. It also invokes
   [`preserve_completed`](../src/services/calendar_interaction.rs), which can
   retain eligible **already-owned completed events from the applied record**,
   and `current_static_windows` refreshes existing desired windows from Task
   state. Thus the ticket's literal "Plan steps only" claim was inaccurate.
   Work stopped and the operator explicitly approved this corrected description
   with existing behavior preserved. Neither path adopts foreign listed events;
   approval executes only the stored preview's operations. Test 4 also checks
   empty-plan preview/apply cannot create a duplicate recurring instance.
4. **Repair:** [`repair`](../src/services/calendar_reconcile.rs) filters observed
   IDs to existing applied ownership. The
   [service](../src/services/calendar_reconciliation_service.rs) retains foreign
   and unrecorded conflicts, never calls Google, and never adopts them. This
   algorithm is unchanged. Existing reconciliation tests remain green, and both
   foreign UI groups contain no repair control; the existing whole-record repair
   control continues to address missing/drifted only.
5. **Normalize:** [`normalize_observed`](../src/services/calendar_capture.rs)
   matches exact `external_id` and restores an owned Task identity if present;
   it canonicalizes timestamps without validating or rewriting the external ID.
   Recurring IDs survive the service path exercised by tests 2–4.

No other downstream discrepancy was found. The backend shares one refusal helper
between capture and reconciliation. Foreign classification is unchanged, but its
message carries the refusal reason when appropriate. Reconcile also emits that
same diagnostic through its **existing** diagnostics array, allowing the UI to
match the reason and form the second group without duplicating Rust's ID rule or
adding fields/routes. Capture displays its existing diagnostic rendering.

## Isolation and data handling

All unit/integration tests, Clippy, TypeScript and UI build commands ran under the
existing local seccomp wrapper that denies IPv4 and IPv6 sockets; Cargo also used
`--locked --offline`. Calendar test responses are invented fixtures decoded by
the production wire parser into `RecordingCalendarApi`; storage is in memory.
No test contacted Google, opened a credential or used a real Calendar account,
calendar ID, event ID or title. The required contract script is the separate,
explicit loopback exception: it builds offline, starts a scrubbed process using
an isolated temporary store under `/tmp`, drives only `127.0.0.1`, and cleans up.
Public Google documentation was read for ID rules and example shapes as required
by the preconditions; no Calendar account or API was queried. Git pushes are the
operator-authorized network exception.

All **task-data files** read or written were under `~/ubu-phase1b` or `/tmp`.
No operator-home token files were listed, read, written or moved. Acceptance
artifacts remained unopened and locally excluded by operator authorization;
their names and contents were excluded from committed changes. This is not a
claim that compiler/runtime/Git processes never read system libraries, installed
toolchains, caches or Git configuration outside those directories: ordinary
execution necessarily uses those. No runtime memory/concurrency caps were added.

## Release inventory

The after-pin inventory is reproduced below with only the self-referential
orchestrator HEAD and PINNED SHA replaced by `<ORCH-F>`. Final verification compares
this block byte-for-byte with actual `show-revs.sh` output after that substitution.
The unmodified output is included in devshell's `P1B-44_PINS.md`. Devshell itself
and quick-ubu are not rows in this script; their clean states are checked
separately. All nine displayed repositories are OK and clean.

```text
Recorded R_* baseline: post-O20 R_orchestrator, post-GA2 R_adapter, post-S17 R_schemas, post-C12 R_core, post-ST7 R_store

REPO                     BRANCH         HEAD      SIG                 TREE   PINNED    STATUS
----                     ------         ----      ---                 ----   ------    ------
ubu_design               main           f7c4a1db  signed-ok           clean  f7c4a1db  OK
ubu_schemas              main           4974166a  signed-ok           clean  4974166a  OK
ubu_core                 main           c77c0a2d  signed-ok           clean  c77c0a2d  OK
ubu_store                main           7b24cd82  signed-ok           clean  7b24cd82  OK
ubu_github_adapter       main           4c7e3b6d  signed-ok           clean  4c7e3b6d  OK
ubu_planning_kernel      main           84b6d0d9  signed-ok           clean  84b6d0d9  OK
ubu_orchestrator         p1b-44-foreign-tolerance <ORCH-F>  unsigned            clean  <ORCH-F>  OK
ubu_ui                   p1b-44-foreign-tolerance 2796a069  unsigned            clean  2796a069  OK
ubu_brand                main           faf2005a  signed-ok           clean  faf2005a  OK
```

## Judgment calls and literal readings

No disagreement with the eleven judgment calls. The accepted correction to the
five-link precondition is documented above. The following ambiguities were
resolved without expanding scope:

- "No pin moves" means no **dependency** pins; the expressly required final
  devshell inventory pins for the two changed repositories do move.
- "Branch in every repo" applies to the three changed repositories; read-only
  repositories remain untouched.
- Observation sees every **representable timed event**. Existing wire validation
  stays unchanged; this does not add all-day support or stricter colour-string,
  transparency-value or reminder validation beyond the existing parser.
- The new foreign group and count cover ID-based ownership refusals in the
  bounded observation, not malformed or all-day items omitted by parsing.
  Ordinary foreign events can still fail later title/window admission checks.
- Reconcile needs the backend refusal before capture runs. Section B therefore
  also supplies that reason through existing reconciliation messages/diagnostics.
  No ownership classification, apply algorithm or repair behavior changes.
- The shared diagnostic retains the `capture_event_not_ownable` code in both
  stages, so the UI consumes backend evidence without inventing a second rule.
- The final pin commit is required but unlettered; self-referential revisions are
  represented as described above and resolved in the final delivery.
- Local acceptance-file excludes were explicitly authorized. Only local Git
  metadata changed; no filenames, credential data or hashes were published.
- The old bootstrap paragraph saying recurring instances become one-off Tasks
  was corrected. Its stale "no Routines screen" sentence was also corrected to
  reflect accepted P1B-43. The old approval warning is already labelled resolved.
- The broad filesystem statement concerns task data, with the tool/runtime
  qualification above. Required loopback verification and public-documentation
  reading are distinct from the network-denied test suites.

## Known limits, verbatim

1. **Recurring events are not imported.** They are seen, named and refused.
2. **An uncaptured commitment does not occupy capacity**, so the planner may place work over it. Reported, not fixed, and the most important limit in this list.
3. **All-day events are still unsupported**, unchanged.
4. **Capture still claims a foreign event the moment it runs**, so reconcile must run first to see anything as foreign.
5. **No Review screen**, and nothing produces advisory candidates yet. That is the last large gap before the switch.
6. **The Calendar export records no approver**, as P1B-43's runner noted.
7. **The Routines screen cannot clear an override**, though the orchestrator has the `DELETE`.
8. **"Applied events" on the approve result is the size of the applied record**, not a count of events pushed in that run, and the wording still invites the opposite reading.
