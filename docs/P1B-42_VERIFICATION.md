# P1B-42 verification

## Landing order and scope

All changed repositories use `p1b-42-palette-authority`. Track B's A–D was pushed
before Track A's E–G. Section H documents both repositories, with one H commit in
each; the final orchestrator H is pushed before UI H, and devshell I lands last.
No force-push. No dependency or Cargo pin moved, no manifest changed and no new
dependency was added. Section I updates only the explicit checkout inventory.

| Final landing order | Repository | Exact repository-qualified revision |
|---|---|---|
| 1 | ubu-orchestrator | `origin/p1b-42-palette-authority`, commit subject `P1B-42 H: document Settings and calendar bootstrap verification` |
| 2 | ubu-ui | `origin/p1b-42-palette-authority`, commit subject `P1B-42 H: document the Colours card in Setup` |
| 3 | ubu-devshell | `origin/p1b-42-palette-authority`, commit subject `P1B-42 I: record the final palette authority revisions` |

The tested orchestrator functional revision D is `a068020df60fdf9578837225873691ac059de431`;
the tested UI functional revision G is `6dd2e8170bd4f826013e7239002d141b245560ae`.
H changes documentation only. This report uses qualified branch/subject
references because its own H commit hash and the later I hash that pins it cannot
be embedded without a circular hash dependency. The final response supplies the
three resolved SHAs, also captured in the local
[landing record](../../.p1b-42-results/landing.json). Final inventory output below
normalizes only the two H hashes and is checked against the actual post-I output.

One commit per lettered section **per affected repository**, including H in both
repositories as its instructions require. UI and devshell use the existing
Co-Authored-By trailer; orchestrator uses none. Signing is disabled per commit;
no signing key is requested or inspected. All final trees are clean. All seven
specified read-only repositories, and the existing brand checkout, retain their
baseline revisions. The Google Calendar screen, navigation source, Quick UbU
import route and export authority gate are unchanged.

## Test counts, checks and lockfiles

| Check | Before | After | Delta |
|---|---:|---:|---:|
| Orchestrator tests | 337 | 348 | +11 |
| UI tests | 30 | 35 | +5 |
| Clippy emitted warnings | 16 | 16 | 0 |
| Clippy distinct warnings | 9 | 9 | 0 |
| UI path constants verified live | 24 | 27 | +3 |

Orchestrator: `cargo test --locked --offline` passes. A, B and C each passed all
337 existing tests before their commit. D adds nine `setting_authoring` tests and
two capture tests. UI: all 30 existing tests pass at E and F; G adds five for 35.
`npx --no-install tsc --noEmit` and `npm run build` pass. No Rust concurrency or
memory cap was introduced.

Clippy command: `cargo clippy --locked --offline --all-targets --message-format=json`.
Count only JSON `compiler-message` records with `message.level == "warning"` and
a target source path under this orchestrator checkout. Emitted counts include
lib/lib-test repeats. Distinct warnings deduplicate on diagnostic code, message
and primary `(file_name, line_start, column_start)` spans. Both methods give a
zero delta; the nine distinct warnings are pre-existing. Raw JSON and counts
are in `.p1b-42-results/clippy-before.jsonl`, `clippy-after.jsonl`, and `counts.json`.

All three locks are byte-identical to their baseline Git blobs. SHA-256:

| Lockfile | SHA-256 before and after |
|---|---|
| `ubu-orchestrator/Cargo.lock` | `e7a0ecf2a14949e5d106ffc3d744605225c56c6ca9d049ff5e698790c6641a15` |
| `ubu-ui/package-lock.json` | `f17aa89a8b2770b7c28e1aa38fe5a9a1b71c938f58a6bd87cb3666fd448d330f` |
| `ubu-ui/src-tauri/Cargo.lock` | `4225e62a5930e8d9347d34b2454eaccf39d2738a7a129cba3f2c5bdfce3d5f56` |

The legacy `static_category` tests now read the palette through `from_pool` after
removal of the AppState field; their assertions remain. One existing capture
assertion changes from an empty diagnostic list to `capture_colour_unmapped`,
because its synthetic colour `99` is precisely the previously silent case being
fixed. Its transparency, admission and projection assertions are preserved.
Existing UI mocks add a synthetic `/settings` response when opening Setup; the
Calendar screen and approval implementation are not edited.

## Verbatim HTTP evidence

The following are unmodified JSON output values emitted by the synthetic
in-process tests, including randomly generated synthetic Setting IDs. All
assertions execute against the real router/store; no HTTP socket is involved.

### Test 1: eleven defaults, origin default

```json
{"inverse":[{"categories":["entertainment"],"color_id":"1","status":"mapped"},{"categories":["grocery"],"color_id":"2","status":"mapped"},{"categories":["personal"],"color_id":"3","status":"mapped"},{"categories":["undefined"],"color_id":"4","status":"mapped"},{"categories":["relationship"],"color_id":"5","status":"mapped"},{"categories":["business"],"color_id":"6","status":"mapped"},{"categories":["commute"],"color_id":"7","status":"mapped"},{"categories":["location"],"color_id":"8","status":"mapped"},{"categories":["work"],"color_id":"9","status":"mapped"},{"categories":["education_house"],"color_id":"10","status":"mapped"},{"categories":["committed"],"color_id":"11","status":"mapped"}],"palette":[{"category":"business","color_id":"6","origin":"default"},{"category":"committed","color_id":"11","origin":"default"},{"category":"commute","color_id":"7","origin":"default"},{"category":"education_house","color_id":"10","origin":"default"},{"category":"entertainment","color_id":"1","origin":"default"},{"category":"grocery","color_id":"2","origin":"default"},{"category":"location","color_id":"8","origin":"default"},{"category":"personal","color_id":"3","origin":"default"},{"category":"relationship","color_id":"5","origin":"default"},{"category":"undefined","color_id":"4","origin":"default"},{"category":"work","color_id":"9","origin":"default"}],"schema_version":"ubu.orchestrator.setting.v1","settings":[]}
```

### Test 2: native Setting overrides the default

```json
{"inverse":[{"categories":["entertainment"],"color_id":"1","status":"mapped"},{"categories":["grocery"],"color_id":"2","status":"mapped"},{"categories":["personal"],"color_id":"3","status":"mapped"},{"categories":["undefined"],"color_id":"4","status":"mapped"},{"categories":["relationship"],"color_id":"5","status":"mapped"},{"categories":["business","work"],"color_id":"6","status":"collision"},{"categories":["commute"],"color_id":"7","status":"mapped"},{"categories":["location"],"color_id":"8","status":"mapped"},{"categories":[],"color_id":"9","status":"unmapped"},{"categories":["education_house"],"color_id":"10","status":"mapped"},{"categories":["committed"],"color_id":"11","status":"mapped"}],"palette":[{"category":"business","color_id":"6","origin":"default"},{"category":"committed","color_id":"11","origin":"default"},{"category":"commute","color_id":"7","origin":"default"},{"category":"education_house","color_id":"10","origin":"default"},{"category":"entertainment","color_id":"1","origin":"default"},{"category":"grocery","color_id":"2","origin":"default"},{"category":"location","color_id":"8","origin":"default"},{"category":"personal","color_id":"3","origin":"default"},{"category":"relationship","color_id":"5","origin":"default"},{"category":"undefined","color_id":"4","origin":"default"},{"category":"work","color_id":"6","origin":"setting"}],"schema_version":"ubu.orchestrator.setting.v1","settings":[{"authority_source":"user","id":"setting_01a0e83699427ca08d661e539357a7c2","name":"calendar.color.work","value":"6","version":1}]}
```

This test also checks user authority, exact native provenance without source,
server-generated `setting_` ID, update of the same ID from version 1 to 2, and one
canonical row rather than a second Setting.

### Test 3: file seed before the Setting

```json
{"inverse":[{"categories":["entertainment"],"color_id":"1","status":"mapped"},{"categories":["grocery"],"color_id":"2","status":"mapped"},{"categories":["personal"],"color_id":"3","status":"mapped"},{"categories":["undefined"],"color_id":"4","status":"mapped"},{"categories":["relationship"],"color_id":"5","status":"mapped"},{"categories":["business","work"],"color_id":"6","status":"collision"},{"categories":["commute"],"color_id":"7","status":"mapped"},{"categories":["location"],"color_id":"8","status":"mapped"},{"categories":[],"color_id":"9","status":"unmapped"},{"categories":["education_house"],"color_id":"10","status":"mapped"},{"categories":["committed"],"color_id":"11","status":"mapped"}],"palette":[{"category":"business","color_id":"6","origin":"default"},{"category":"committed","color_id":"11","origin":"default"},{"category":"commute","color_id":"7","origin":"default"},{"category":"education_house","color_id":"10","origin":"default"},{"category":"entertainment","color_id":"1","origin":"default"},{"category":"grocery","color_id":"2","origin":"default"},{"category":"location","color_id":"8","origin":"default"},{"category":"personal","color_id":"3","origin":"default"},{"category":"relationship","color_id":"5","origin":"default"},{"category":"undefined","color_id":"4","origin":"default"},{"category":"work","color_id":"6","origin":"file"}],"schema_version":"ubu.orchestrator.setting.v1","settings":[]}
```

After the Setting, on the same process/state:

```json
{"inverse":[{"categories":["entertainment"],"color_id":"1","status":"mapped"},{"categories":["grocery","work"],"color_id":"2","status":"collision"},{"categories":["personal"],"color_id":"3","status":"mapped"},{"categories":["undefined"],"color_id":"4","status":"mapped"},{"categories":["relationship"],"color_id":"5","status":"mapped"},{"categories":["business"],"color_id":"6","status":"mapped"},{"categories":["commute"],"color_id":"7","status":"mapped"},{"categories":["location"],"color_id":"8","status":"mapped"},{"categories":[],"color_id":"9","status":"unmapped"},{"categories":["education_house"],"color_id":"10","status":"mapped"},{"categories":["committed"],"color_id":"11","status":"mapped"}],"palette":[{"category":"business","color_id":"6","origin":"default"},{"category":"committed","color_id":"11","origin":"default"},{"category":"commute","color_id":"7","origin":"default"},{"category":"education_house","color_id":"10","origin":"default"},{"category":"entertainment","color_id":"1","origin":"default"},{"category":"grocery","color_id":"2","origin":"default"},{"category":"location","color_id":"8","origin":"default"},{"category":"personal","color_id":"3","origin":"default"},{"category":"relationship","color_id":"5","origin":"default"},{"category":"undefined","color_id":"4","origin":"default"},{"category":"work","color_id":"2","origin":"setting"}],"schema_version":"ubu.orchestrator.setting.v1","settings":[{"authority_source":"user","id":"setting_01a0e83699427ca08d661e79970455ea","name":"calendar.color.work","value":"2","version":1}]}
```

The synthetic file is removed after startup, so these requests also demonstrate
that the file remains a startup seed rather than a request-time file dependency.
Test 6 verifies deletion returns to both file and default origins and removes a
Setting-only category. Existing malformed-file startup tests continue to pass.

### Test 4: invalid colour rejection, no admission

```json
{"diagnostics":[{"code":"setting_invalid_color","message":"Colour must be a string with one of the allowed ids: 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11"}],"error":"Colour must be a string with one of the allowed ids: 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11"}
```

The test also rejects numeric/boolean/null values, malformed colour strings,
missing/unknown schema versions and caller-supplied authority. Test 5 rejects
unknown namespaces and an empty category suffix.

### Test 7: the next preview changes without restart

```json
{"after_color_id":"2","before_color_id":"9","regenerated":false,"restarted":false,"same_plan":true,"same_state":true}
```

The actual projected `events[0].color_id` changes verbatim from **`"9"`** to
**`"2"`**. The test uses the same AppState and same admitted Plan ID, does not
restart or regenerate, and asserts no calls to the Calendar recorder. The
preview resolves current configuration without rewriting the admitted Plan.

Test 8 swaps work to colour 1 and entertainment to 9, verifies the unchanged
`inverse()` API maps 1 back to work, and captures a foreign event into that
category. An owned Dynamic event remains initially uncoloured, then completing
it through mock phone colouring still works. The captured Static Task remains
active. This preserves P1B-33's placement partition and completion semantics.

### Test 9: inverse includes a collision and an unmapped allowed colour

```json
[{"categories":["entertainment","work"],"color_id":"1","status":"collision"},{"categories":["grocery"],"color_id":"2","status":"mapped"},{"categories":["personal"],"color_id":"3","status":"mapped"},{"categories":["undefined"],"color_id":"4","status":"mapped"},{"categories":["relationship"],"color_id":"5","status":"mapped"},{"categories":["business"],"color_id":"6","status":"mapped"},{"categories":["commute"],"color_id":"7","status":"mapped"},{"categories":["location"],"color_id":"8","status":"mapped"},{"categories":[],"color_id":"9","status":"unmapped"},{"categories":["education_house"],"color_id":"10","status":"mapped"},{"categories":["committed"],"color_id":"11","status":"mapped"}]
```

Colour 1 lists both entertainment and work as a collision. Allowed colour 9
remains present with an empty category list and status `unmapped`.

### Test 10: unmapped colour is advisory

```json
{"captured_task":{"id":"task_01a0e83698e679a094e326d1b81a1866","occupies_capacity":true,"provenance":{"authority_source":"user","created_at":"2026-09-25T08:00:00Z","source":{"source_id":"ccccc","source_kind":"google_calendar"}},"static_window":{"end":"2026-09-25T12:30:00Z","start":"2026-09-25T12:00:00Z"},"status":"active","title":"Synthetic unmapped-colour appointment"},"response":{"captured":1,"diagnostics":[{"code":"capture_colour_unmapped","message":"Calendar event `ccccc` has unmapped colour `99`; no category assigned; map that colour in Settings to assign a category"}],"moved":0,"resized":0,"schema_version":"ubu.orchestrator.calendar_capture.v1","skipped":0,"unchanged":0,"updated":0}}
```

Verbatim diagnostic:

```text
capture_colour_unmapped
Calendar event `ccccc` has unmapped colour `99`; no category assigned; map that colour in Settings to assign a category
```

### Test 11: absent colour is advisory

```json
{"captured_task":{"id":"task_01a0e83698e473a0854ac9b207e899c4","occupies_capacity":true,"provenance":{"authority_source":"user","created_at":"2026-09-25T08:00:00Z","source":{"source_id":"ddddd","source_kind":"google_calendar"}},"static_window":{"end":"2026-09-25T12:30:00Z","start":"2026-09-25T12:00:00Z"},"status":"active","title":"Synthetic uncoloured appointment"},"response":{"captured":1,"diagnostics":[{"code":"capture_colour_absent","message":"Calendar event `ddddd` has no colour; no category assigned"}],"moved":0,"resized":0,"schema_version":"ubu.orchestrator.calendar_capture.v1","skipped":0,"unchanged":0,"updated":0}}
```

Verbatim diagnostic:

```text
capture_colour_absent
Calendar event `ddddd` has no colour; no category assigned
```

Both tests assert **captured: 1**, **skipped: 0**, a concrete Static window,
`occupies_capacity: true`, no category, and only a recorder ListEvents call.
The existing ambiguous-colour diagnostic text is unchanged. Whole-repository
searches, including tests, were recorded before and after the diagnostic edits.

## UI evidence

Tests 31–35 use only `vi.mock("@tauri-apps/plugin-http", ...)` and cover all eleven
category rows with swatches, IDs and origins; a successful PUT and list reload;
the exact allowed-ID rejection; DELETE and fallback origin; and inverse rows:

```text
Collision: entertainment, work — no category assigned.
Unmapped — no category assigned.
Changes take effect on the next Calendar preview and the next capture, with no restart. Check the inverse mapping before bootstrapping from your calendar.
```

All eleven allowed inverse colours are asserted present. Existing 30 tests remain,
including the P1B-41 Calendar mocks; adding `/settings` to those Setup mocks does
not constitute Google acceptance.

## Isolation and fixture locations

No test contacted Google, read a credential, or used real calendar/Quick UbU data.
Rust tests invoke Axum's router in-process with an in-memory store and
`RecordingCalendarApi`; all capture/approval requests use mock export mode. The
new fixture titles explicitly start with Synthetic, and IDs are synthetic.
New backend fixtures live in `tests/setting_authoring.rs` and the two added tests
in `tests/calendar_capture.rs`. UI fixtures live in
`ubu-ui/tests/fixtures/settings.ts` and `ubu-ui/tests/colours.test.tsx`.

Full Rust and UI suites, Clippy, code generation and UI production builds use the
existing seccomp offline wrapper denying IPv4/IPv6 socket creation. Cargo uses
`--locked --offline`; no dependencies are fetched. The shared UI setup throws on
global fetch, and the new plugin mocks assert unexpected requests are absent.
The only local HTTP checks are the separately required contract script, which
runs a scrubbed, credential-free child with fresh `/tmp` state and only loopback
requests. Git is the standing explicit network exception.

All task source/input, fixture, editing and verification-artifact reads/writes
stay inside `/home/sean/ubu-phase1b` and `/tmp`. No operator home data or OAuth
pickle was listed, opened, moved or written. This is a task-data boundary;
installed tools still load their own libraries/caches and Git configuration or
authentication. No claim of an OS-wide filesystem-access audit is made. No
credential material or real operator store contents were inspected or recorded.
The pre-existing ignored runtime store/registration files remain untouched.

## Contract check output

Before, on the supplied baselines:

```text
build: cargo build --locked --offline in /home/sean/ubu-phase1b/ubu-orchestrator
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.22s
built: /home/sean/ubu-phase1b/ubu-orchestrator/target/debug/ubu_orchestrator
start: orchestrator on ephemeral port 43199, store in /tmp/ubu-contract-check.3xkxLV
defaults:
  ubu-ui            DEFAULT_ORCHESTRATOR_PORT = 7878
  ubu-orchestrator  UBU_ORCHESTRATOR_PORT unwrap_or = 7878
  the two defaults agree
requests against http://127.0.0.1:43199:
  200 GET http://127.0.0.1:43199/health
  201 POST http://127.0.0.1:43199/task
  200 GET http://127.0.0.1:43199/tasks?schema_version=ubu.orchestrator.task_read.v1&status=active
  200 PATCH http://127.0.0.1:43199/task/task_01a0e8280ab57213a9ea95e3fce7f2b7
  200 POST http://127.0.0.1:43199/planning/generate
  200 GET http://127.0.0.1:43199/next-action?schema_version=ubu.orchestrator.next_action.v1
paths in the live /openapi.json:
  200 GET http://127.0.0.1:43199/openapi.json
  ok      BOOTSTRAP_SEED_PATH = /bootstrap/seed
  ok      CALENDAR_APPROVE_PATH = /projection/calendar/approve
  ok      CALENDAR_CAPTURE_PATH = /projection/calendar/capture
  ok      CALENDAR_CURRENT_PATH = /calendar/current
  ok      CALENDAR_PREVIEW_PATH = /projection/calendar/preview
  ok      CALENDAR_RECONCILE_PATH = /projection/calendar/reconcile
  ok      CALENDAR_REPAIR_PATH = /projection/calendar/reconcile/{reconciliation_id}/repair
  ok      DESKTOP_TOKEN_PATH = /desktop/session/github-token
  ok      GOOGLE_CALENDAR_SESSION_PATH = /desktop/session/google-calendar
  ok      HEALTH_PATH = /health
  ok      NEXT_ACTION_PATH = /next-action
  ok      PLANNING_GENERATE_PATH = /planning/generate
  ok      PLANNING_RECALCULATE_PATH = /planning/recalculate
  ok      PREFERENCE_CREATE_PATH = /preference
  ok      PREFERENCE_LIST_PATH = /preferences
  ok      PREFERENCE_PATH = /preference/{preference_id}
  ok      PROJECTION_ACCEPT_EXTERNAL_PATH = /projection/reconciliation/accept-external
  ok      PROJECTION_APPROVE_PATH = /projection/approve
  ok      PROJECTION_PREVIEW_PATH = /projection/preview
  ok      PROJECTION_RECONCILE_PATH = /projection/reconcile
  ok      RECORD_TASK_ACTION_PATH = /task/{task_id}/action
  ok      TASK_CAPTURE_PATH = /task
  ok      TASK_LIST_PATH = /tasks
  ok      TASK_PATH = /task/{task_id}
PASS: ubu-ui contract check: defaults agree on 7878, 7 requests succeeded, 24 of 24 path constants are live
stopped: orchestrator pid 201383
removed: /tmp/ubu-contract-check.3xkxLV
```

After the Settings routes and UI constants:

```text
build: cargo build --locked --offline in /home/sean/ubu-phase1b/ubu-orchestrator
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.21s
built: /home/sean/ubu-phase1b/ubu-orchestrator/target/debug/ubu_orchestrator
start: orchestrator on ephemeral port 45087, store in /tmp/ubu-contract-check.8vju7a
defaults:
  ubu-ui            DEFAULT_ORCHESTRATOR_PORT = 7878
  ubu-orchestrator  UBU_ORCHESTRATOR_PORT unwrap_or = 7878
  the two defaults agree
requests against http://127.0.0.1:45087:
  200 GET http://127.0.0.1:45087/health
  201 POST http://127.0.0.1:45087/task
  200 GET http://127.0.0.1:45087/tasks?schema_version=ubu.orchestrator.task_read.v1&status=active
  200 PATCH http://127.0.0.1:45087/task/task_01a0e83a019374a3b3e4015da1807d96
  200 POST http://127.0.0.1:45087/planning/generate
  200 GET http://127.0.0.1:45087/next-action?schema_version=ubu.orchestrator.next_action.v1
paths in the live /openapi.json:
  200 GET http://127.0.0.1:45087/openapi.json
  ok      BOOTSTRAP_SEED_PATH = /bootstrap/seed
  ok      CALENDAR_APPROVE_PATH = /projection/calendar/approve
  ok      CALENDAR_CAPTURE_PATH = /projection/calendar/capture
  ok      CALENDAR_CURRENT_PATH = /calendar/current
  ok      CALENDAR_PREVIEW_PATH = /projection/calendar/preview
  ok      CALENDAR_RECONCILE_PATH = /projection/calendar/reconcile
  ok      CALENDAR_REPAIR_PATH = /projection/calendar/reconcile/{reconciliation_id}/repair
  ok      DESKTOP_TOKEN_PATH = /desktop/session/github-token
  ok      GOOGLE_CALENDAR_SESSION_PATH = /desktop/session/google-calendar
  ok      HEALTH_PATH = /health
  ok      NEXT_ACTION_PATH = /next-action
  ok      PLANNING_GENERATE_PATH = /planning/generate
  ok      PLANNING_RECALCULATE_PATH = /planning/recalculate
  ok      PREFERENCE_CREATE_PATH = /preference
  ok      PREFERENCE_LIST_PATH = /preferences
  ok      PREFERENCE_PATH = /preference/{preference_id}
  ok      PROJECTION_ACCEPT_EXTERNAL_PATH = /projection/reconciliation/accept-external
  ok      PROJECTION_APPROVE_PATH = /projection/approve
  ok      PROJECTION_PREVIEW_PATH = /projection/preview
  ok      PROJECTION_RECONCILE_PATH = /projection/reconcile
  ok      RECORD_TASK_ACTION_PATH = /task/{task_id}/action
  ok      SETTINGS_LIST_PATH = /settings
  ok      SETTING_DELETE_PATH = /setting/{name}
  ok      SETTING_PUT_PATH = /setting/{name}
  ok      TASK_CAPTURE_PATH = /task
  ok      TASK_LIST_PATH = /tasks
  ok      TASK_PATH = /task/{task_id}
PASS: ubu-ui contract check: defaults agree on 7878, 7 requests succeeded, 27 of 27 path constants are live
stopped: orchestrator pid 238273
removed: /tmp/ubu-contract-check.8vju7a
```

The count increases **24 → 27**, exactly three constants. PUT and DELETE share
one URI template; they have separate operation-specific constants as requested.
There are two distinct new URI templates, three method/route operations. The
orchestrator's committed OpenAPI output is regenerated with its local example,
then copied through the existing offline UI generation script; each constant
`satisfies GeneratedPath`. No devshell script change is needed.

Section I repeats the contract check after both repositories are pushed and the
inventory is updated. Its complete output is in
[contract-after-I.log](../../.p1b-42-results/contract-after-I.log), again passing
27 of 27. These checks assert route availability and existing contracts, not
Google account access or Tauri rendering.

## Post-I inventory output

Only the two H commit hashes are normalized to avoid self-reference. The final
audit checks this block byte-for-byte against the actual post-I output with those
two substitutions. Full raw output and concrete revisions are in
[show-revs-after-I.log](../../.p1b-42-results/show-revs-after-I.log).

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
ubu_orchestrator         p1b-42-palette-authority <ORCH-H>  unsigned            clean  <ORCH-H>  OK
ubu_ui                   p1b-42-palette-authority <UI-H>    unsigned            clean  <UI-H>    OK
ubu_brand                main           faf2005a  signed-ok           clean  faf2005a  OK
```

Every listed repository is OK and clean. Quick UbU, which is not listed by this
script, is separately verified unchanged and clean. Devshell is checked clean
following its own commit. Neither read-only repository HEADs nor application
dependency pins move.

## Judgment calls, findings and literal readings

The fourteen judgment calls are implemented, with the following scope and
contract qualifications recorded rather than silently changing unrelated code:

- **Judgment call 13 leaves an acceptance blocker outstanding.** The P1B-41
  `approveCalendar` wrapper sends `authority_source: "user"`. The existing
  `ubu-core/src/projection/legitimizer.rs` export gate rejects user-equivalent
  authority and requires `automation_worker`; the Calendar apply path checks the
  same authority on its permit. This was exposed by the new completion test's
  initial mock approval, then verified against the code. Correct backend test
  exports use `automation_worker`. The Calendar surface/wrapper and gate remain
  unchanged as explicitly required. Recommend a separate correction before
  P1B-41 acceptance; those mocked UI tests alone did not catch this mismatch.
- The ticket's environment-only horizon procedure is conditional. A valid latest
  stored Calendar window takes precedence in `resolve_time_window`; only without
  one does `UBU_PLANNING_HORIZON_SECONDS` control the range. The bootstrap guide
  quotes the bounded-observation comment, describes widening and restoring the
  environment value for the fresh-workspace fallback, and states this precedence.
  No resolver or stored-window change is made.
- The heading says “one new diagnostic”, but call 8 and section C explicitly name
  two. Both are added, distinct from the unchanged ambiguous diagnostic.
- “Three paths” is read as three operation-specific constants: GET /settings,
  PUT /setting/{name}, DELETE /setting/{name}; no artificial third URI is added.
- Startup validation/file seeding is retained in the store. The constructor and
  seed support land with A because GET must report origin and effective values;
  B removes the AppState field and switches consumers, adding fresh preview
  colour resolution to meet test 7 without regeneration. The old planning-service
  reference at line 2090 is a helper signature, not another request handler.
- The cited `ubu-store/tests/admit_object.rs` contains a Task example, not a
  Setting test. Existing Setting admission is in the store admission dispatcher
  and orchestrator bootstrap service. New tests exercise that real admission
  path, including the ticket's requested native provenance without source.
  The core Setting schema itself does not declare provenance; the existing
  runtime admission path accepts it, as native bootstrap already does. No
  read-only schema/core/store change is made.
- “No pin moves” is read as no dependency pin changes. The explicitly requested
  devshell inventory updates in I are made last.
- The report references final commits by repository-qualified branch/subject and
  supplies final concrete hashes in the landing record/response. One H commit is
  made in each repository containing H's requested documentation. Actual post-I
  output is captured externally and compared against the normalized block above,
  avoiding an extra documentation commit or force-push.
- The new unmapped/absent diagnostics do not retroactively categorise previously
  captured, now-owned Tasks. The bootstrap guide makes the “check first” order
  explicit; foreign capture behaviour is otherwise unchanged.

No other disagreement with the fourteen decisions. The operator's six P1B-42
acceptance steps remain outstanding, **after** the outstanding six P1B-41 steps.
The Google round trip remains unverified by the agent. No dummy or real account
was accessed to perform acceptance.

## Known limits (verbatim)

1. **No Routines screen.** Authoring exists over HTTP since P1B-38, and the calendar bootstrap makes it more necessary, not less, because recurrence cannot survive the calendar. Next ticket.
2. **No Review screen**, and nothing produces advisory candidates yet.
3. **A bootstrap captures only what the planning horizon covers.** Widening it for the run is the control, and nothing automates that.
4. **Captured Tasks are Static and pinned.** Nothing infers that a commitment was really Dynamic work.
5. **Recurrence, Preferences, dependencies, bounded `after` relations and `establishes`/`requires` do not survive the calendar**, because a calendar event does not carry them.
6. **Settings are colour-only for now.** The namespace is general; `setting_unknown_name` rejects everything else until each new name is given its own validation.
7. **The palette is read per request with no caching.**
8. **The two new diagnostics are advisory.** An event with no usable colour is still captured, without a category.
9. **`POST /import/quick-ubu` is untouched and still works over HTTP.** It simply has no screen and is not part of the plan.
10. **P1B-41's Calendar acceptance is still outstanding.**
