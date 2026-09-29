# P1B-45 verification

## Result and acceptance boundary

An explicit SuggestTags run now selects active uncategorized Tasks, sends only
Task IDs and titles as Task data to a configured local model, and enqueues
validated category proposals. Review exposes the durable lifecycle; only operator
admission changes Task category. Setup edits the two advisory Settings.

All automated acceptance uses synthetic fixtures. **Live local-Ollama acceptance
remains for the operator**; the six steps are in [ADVISORY.md](ADVISORY.md#operator-acceptance-live-local-ollama-not-performed-by-automated-tests).
No agent-run test constructs or contacts the live model transport. Calendar
colour acceptance must use a Static Task with a mapped category: Dynamic events
remain uncoloured under the existing partition.

## Revisions and landing order

Baselines: orchestrator `fdec65a`, UI `2796a06`, devshell `379b7c1`.
The three changed repositories use `p1b-45-advisory-producers`; read-only siblings
retain their branches and revisions. Every tree started clean, with the previously
operator-authorized local excludes retained for acceptance artifacts. Those
artifacts were not opened, and neither names nor contents were committed.

Backend sections A, B, D, C and E were pushed first at
`ed1c69d56de40667c0dcc908a7c9c6deb84594a0`, before any UI push. D precedes C because the
run route depends on the producer. Final tips land in this order:

| Order | Repository | Final pushed revision |
| --- | --- | --- |
| 1 | ubu-orchestrator | The section J commit containing this report on `p1b-45-advisory-producers` |
| 2 | ubu-ui | `4346ce4010e5e78defcd31527a7a408a98fe71f5` (section J) |
| 3 | ubu-devshell | The section K pin commit on `p1b-45-advisory-producers` |

Concrete final SHAs and the unmodified inventory are recorded in
[devshell's P1B-45 pin report](https://github.com/UbU-project/ubu-devshell/blob/p1b-45-advisory-producers/docs/P1B-45_PINS.md)
and the delivery response. A report cannot include its own commit hash and a
later commit hash that pins it without a reference cycle. Section K verifies the
normalized inventory below against actual final output before its commit.

One commit per section: A, B, D, C, E and J in orchestrator; F, G, H, I and J in
UI; K in devshell. All UI and devshell commits carry `Co-Authored-By`; orchestrator
commits do not. Ordinary branch pushes only, no force-push or merge.

**No dependency pin moved.** The two release inventory pins in devshell move
last, as explicitly required by K. All manifests and lockfiles are unchanged.
Read-only baselines: core `c77c0a2`, store `7b24cd8`, schemas `4974166`, planning
kernel `84b6d0d`, GitHub adapter `4c7e3b6`, quick-ubu `9ccc8b8`, design `f7c4a1d`
and brand `faf2005`.

## Checks

| Check | Before | After |
| --- | --- | --- |
| Orchestrator `cargo test --locked --offline` | 360 passed | 370 passed |
| UI `npm test` (Vitest run) | 47 passed | 53 passed |
| Clippy all targets: raw / unique warnings | 16 / 9 | 16 / 9 |
| Contract live path constants | 33 / 33 | 39 / 39 |

Ten new backend tests live in `tests/advisory_producer.rs`; six UI tests (48–53)
in `tests/review.test.tsx`. Existing lifecycle and calendar suites remain green.
**The existing navigation test's route count/name and expected order were updated
from eight to nine to include Review.** This expectation update adds no test.

Clippy command: `cargo clippy --locked --offline --all-targets --message-format=json`.
Count only `compiler-message` JSON records with message level `warning`.
Deduplicate by message text, diagnostic code and primary span (file, start line,
start column), eliminating repeated library/test emissions. Delta is zero raw,
zero unique, with no added or removed unique warnings. Existing warnings remain.
`npx --offline tsc --noEmit` and `npm run build` pass. Devshell adds no unit tests;
its contract check passes seven isolated loopback requests before and after and
is repeated after the pin update.

The three lockfiles were compared byte-for-byte with baseline copies:
orchestrator `Cargo.lock`, UI `package-lock.json`, UI `src-tauri/Cargo.lock`.
All are identical. No dependencies or dependency features were added. No runtime
memory or concurrency caps were introduced.

Section A passed the nine Setting tests. B passed all-target compilation and the
12 existing advisory controller tests; D and C also passed those 12. E passed the
ten new tests and the full 370-test suite. F, G and H passed the existing 47 UI
tests and TypeScript; I passed 53 and TypeScript. Production build passed before
J. Documentation and inventory changes do not alter tested implementation.

## Verbatim synthetic evidence

### Test 4: exact submission payload

```text
P1B45_TEST4=[{"id":"task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e70","title":"Synthetic lunar teapot 0"}]
```

The test asserts the complete serialized fifteen-field submission, including
that only ID/title pairs occur in `payload`. It checks authority capabilities,
expected result schema, timeout, result size, compute budget, partial-result
policy, empty causal parents/policy versions/digests, provider config, device,
execution context and submission time. These envelope fields stay local.
It separately asserts the HTTP body has exactly `model`, `stream`, `system`,
`prompt` and `format`, and that decoding `prompt` yields precisely this payload.
Fixed instructions/schema and the model name are protocol fields, not additional
Task data. No provenance, compartment labels, objective refs or evidence enter
the request. IDs accompany titles as section D explicitly requires.

### Tests 1, 2 and 6: exact diagnostics

```text
P1B45_TEST1=[{"code":"advisory_unconfigured","message":"advisory.model is not configured; set it in Setup before running SuggestTags"}]
P1B45_TEST2=[{"code":"advisory_unconfigured","message":"advisory.endpoint is not configured; set it in Setup before running SuggestTags"}]
P1B45_TEST6=[{"code":"advisory_connection_failed","message":"The configured local model could not be reached; no candidates were enqueued"}]
P1B45_TEST6=[{"code":"advisory_timeout","message":"The local model exceeded timeout_ms; no candidates were enqueued"}]
P1B45_TEST6=[{"code":"advisory_http_failed","message":"The local model returned an unsuccessful HTTP status; check the configured model; no candidates were enqueued"}]
```

Missing configuration does nothing. Test 6 asserts HTTP 200, failed status and
zero candidates for connection, timeout and unsuccessful HTTP responses. Test 7
checks oversize refusal, bounded chunk accumulation, malformed responses,
unselected IDs, invalid confidence/category and forbidden extra fields.

### Test 10: exact structural evidence

```text
P1B45_TEST1=[{"code":"advisory_unconfigured","message":"advisory.model is not configured; set it in Setup before running SuggestTags"}]0
```

The regression uses `include_str!` to assert that the live module declaration and
factory installation in `main.rs` are guarded by `#[cfg(not(test))]`, and that
neither the library nor service module exports it. `OllamaTransport::new` is
binary-private. The test then uses a normal in-memory AppState without an injected
factory, configures both Settings and a Task, and verifies Run cannot construct
any transport or enqueue a candidate. Other new tests inject only `StubTransport`
behind the existing trait. Pure request/response policy is shared with the live
shell. The production binary is compiled, but live HTTP behavior is deliberately
left for operator acceptance. This structural guard covers the current module
and state wiring; seccomp additionally denies network sockets in the test process
and descendants.

### Test 9: exact durable rejection result

```text
P1B45_TEST9={"candidate_rows":1,"first_enqueued":1,"queue":[],"second_enqueued":0,"suppressed":1}
```

Both runs select the same synthetic Task. The second receives the same proposal
from the stub with a new submission ID, and durable suppression prevents a new
candidate row. Test 8 also proves Run, defer, resurface and reject do not mutate
Task state; explicit ordinary admission sets `category_tag` and the matching tag.
Test 5 proves stable limiting, invalid-bound rejection and duplicate avoidance.

UI tests exercise candidate metadata, all four review routes and reloads,
confirmation before rejection, Run with and without a limit, both missing
Settings with navigation to Setup, both Settings' edit/revert origins and a
non-loopback refusal. The Tauri HTTP plugin is mocked; global fetch is asserted
unused and unexpected requests fail assertions.

## Isolation and data handling

All Cargo tests, Clippy, TypeScript, Vitest and UI build processes run under the
existing local seccomp wrapper denying AF_INET and AF_INET6 sockets, inherited by
children. Cargo uses `--locked --offline`. Fixtures and stores are synthetic;
new backend tests use in-memory stores and injected stubs. No test contacts
Ollama, Google, a real account or a credential. Live transport constructors are
absent from test wiring as detailed above.

The ticket-required `check-ui-contract.sh` is the explicit **separate loopback
exception** to the otherwise network-denied suites: offline build, scrubbed
runtime environment, ephemeral `127.0.0.1` listener, temporary isolated store
under `/tmp`, mock external services, synthetic Tasks, cleanup. It does not call
advisory Run or configure a model. Official public Ollama API documentation was
consulted for the wire format; no model service was queried. Git pushes are the
operator-authorized network exception.

All **task-data files** read or written are under `~/ubu-phase1b` or `/tmp`.
No operator-home token files or excluded acceptance artifacts were read, listed
for contents, written or moved. An outgoing-history audit uses only authorized
local exclude metadata and checks that excluded names and the real event ID in
the ticket do not appear in committed changes. No acceptance names, contents or
hashes are published. The broad filesystem requirement cannot literally exclude
installed toolchains, system libraries, caches and Git configuration read by
ordinary tooling; those runtime reads are the same qualification recorded by
P1B-44. No unrelated home data was explored.

## Contract output before

```text
build: cargo build --locked --offline in /home/sean/ubu-phase1b/ubu-orchestrator
   Compiling ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.71s
built: /home/sean/ubu-phase1b/ubu-orchestrator/target/debug/ubu_orchestrator
start: orchestrator on ephemeral port 40233, store in /tmp/ubu-contract-check.MWftGP
defaults:
  ubu-ui            DEFAULT_ORCHESTRATOR_PORT = 7878
  ubu-orchestrator  UBU_ORCHESTRATOR_PORT unwrap_or = 7878
  the two defaults agree
requests against http://127.0.0.1:40233:
  200 GET http://127.0.0.1:40233/health
  201 POST http://127.0.0.1:40233/task
  200 GET http://127.0.0.1:40233/tasks?schema_version=ubu.orchestrator.task_read.v1&status=active
  200 PATCH http://127.0.0.1:40233/task/task_01a0eaeb5d44739383f2078319e818d3
  200 POST http://127.0.0.1:40233/planning/generate
  200 GET http://127.0.0.1:40233/next-action?schema_version=ubu.orchestrator.next_action.v1
paths in the live /openapi.json:
  200 GET http://127.0.0.1:40233/openapi.json
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
  ok      OBJECTIVE_CREATE_PATH = /objective
  ok      OBJECTIVE_EDIT_PATH = /objective/{objective_id}
  ok      OBJECTIVE_LIST_PATH = /objectives
  ok      OBJECTIVE_READ_PATH = /objective/{objective_id}
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
  ok      ROUTINE_LIST_PATH = /routines
  ok      ROUTINE_OVERRIDE_PATH = /routine/{objective_id}/override/{local_date}
  ok      SETTINGS_LIST_PATH = /settings
  ok      SETTING_DELETE_PATH = /setting/{name}
  ok      SETTING_PUT_PATH = /setting/{name}
  ok      TASK_CAPTURE_PATH = /task
  ok      TASK_LIST_PATH = /tasks
  ok      TASK_PATH = /task/{task_id}
PASS: ubu-ui contract check: defaults agree on 7878, 7 requests succeeded, 33 of 33 path constants are live
stopped: orchestrator pid 284976
removed: /tmp/ubu-contract-check.MWftGP
```

## Contract output after

```text
build: cargo build --locked --offline in /home/sean/ubu-phase1b/ubu-orchestrator
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.22s
built: /home/sean/ubu-phase1b/ubu-orchestrator/target/debug/ubu_orchestrator
start: orchestrator on ephemeral port 36871, store in /tmp/ubu-contract-check.A3E70v
defaults:
  ubu-ui            DEFAULT_ORCHESTRATOR_PORT = 7878
  ubu-orchestrator  UBU_ORCHESTRATOR_PORT unwrap_or = 7878
  the two defaults agree
requests against http://127.0.0.1:36871:
  200 GET http://127.0.0.1:36871/health
  201 POST http://127.0.0.1:36871/task
  200 GET http://127.0.0.1:36871/tasks?schema_version=ubu.orchestrator.task_read.v1&status=active
  200 PATCH http://127.0.0.1:36871/task/task_01a0eb0213cd7900b6a2a64dbd323ca5
  200 POST http://127.0.0.1:36871/planning/generate
  200 GET http://127.0.0.1:36871/next-action?schema_version=ubu.orchestrator.next_action.v1
paths in the live /openapi.json:
  200 GET http://127.0.0.1:36871/openapi.json
  ok      ADVISORY_ADMIT_PATH = /advisory/candidate/{candidate_id}/admit
  ok      ADVISORY_DEFER_PATH = /advisory/candidate/{candidate_id}/defer
  ok      ADVISORY_QUEUE_PATH = /advisory/queue
  ok      ADVISORY_REJECT_PATH = /advisory/candidate/{candidate_id}/reject
  ok      ADVISORY_RESURFACE_PATH = /advisory/candidate/{candidate_id}/resurface
  ok      ADVISORY_RUN_PATH = /advisory/run
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
  ok      OBJECTIVE_CREATE_PATH = /objective
  ok      OBJECTIVE_EDIT_PATH = /objective/{objective_id}
  ok      OBJECTIVE_LIST_PATH = /objectives
  ok      OBJECTIVE_READ_PATH = /objective/{objective_id}
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
  ok      ROUTINE_LIST_PATH = /routines
  ok      ROUTINE_OVERRIDE_PATH = /routine/{objective_id}/override/{local_date}
  ok      SETTINGS_LIST_PATH = /settings
  ok      SETTING_DELETE_PATH = /setting/{name}
  ok      SETTING_PUT_PATH = /setting/{name}
  ok      TASK_CAPTURE_PATH = /task
  ok      TASK_LIST_PATH = /tasks
  ok      TASK_PATH = /task/{task_id}
PASS: ubu-ui contract check: defaults agree on 7878, 7 requests succeeded, 39 of 39 path constants are live
stopped: orchestrator pid 326896
removed: /tmp/ubu-contract-check.A3E70v
```

Exactly six UI path constants were added: queue, admit, reject, defer, resurface,
and Run. Only Run is a new backend route. The existing detail GET remains
unchanged and needs no new UI constant. OpenAPI was regenerated and copied into
the UI through its existing offline codegen script.

## Release inventory after K

The final output below replaces only the self-referential orchestrator HEAD and
PINNED hashes with `<ORCH-J>`. Section K compares this block byte-for-byte to the
actual after-pin output after that substitution; the unmodified output goes into
the devshell pin report. All nine rows must be OK and clean. Devshell and quick-ubu
are absent from this script and are checked separately for clean state.

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
ubu_orchestrator         p1b-45-advisory-producers <ORCH-J>  unsigned            clean  <ORCH-J>  OK
ubu_ui                   p1b-45-advisory-producers 4346ce40  unsigned            clean  4346ce40  OK
ubu_brand                main           faf2005a  signed-ok           clean  faf2005a  OK
```

## Judgment calls and literal readings

No disagreement with the fifteen judgment calls. The following details reconcile
the prompt with the existing implementation without changing those boundaries:

- The factory is executable wiring for the **existing** `AdvisoryTransport` trait,
  not a second transport interface or worker protocol. It has no test fallback.
- Five existing routes means queue plus four review actions for section F. The
  backend also already has a detail GET, retained unchanged. Six new UI constants
  therefore produce exactly 33 → 39.
- Existing `add_tag` only appends to tags. SuggestTags needs `category_tag`, so its
  normalized Tag proposal uses `set_category` through the existing admission
  path. Existing add-tag behavior is preserved; core/store/schema repos do not
  change. A current, conflicting category is refused on admission.
- The store's active queue excludes deferred candidates. The HTTP response adds
  `deferred_candidates` and `target_titles` while preserving `candidates`, so
  Review can offer resurface without changing the store's query contract.
- Existing review action requests have no schema-version field. Their bodies are
  preserved; Run has the required new version and candidate version stays `1.0`.
- Endpoint means a literal HTTP origin with a nonzero numeric port, no suffix;
  no implicit default, hostname resolution, redirect or proxy is permitted.
- Quick UbU uses unpinned/tagless selection, history and a batch of 25. The more
  specific section D is followed here: active and uncategorized, including
  Static Tasks and Tasks with other tags, stable ID order, default 5, maximum 25.
- Declared CPU/memory budget is metadata, not a quota on the separate model
  process. HTTP timeout and response-size bounds are enforced by the transport.
- Only IDs/titles means only those **Task data** fields. Fixed instructions, output
  schema and configured model name necessarily accompany them on the wire.
  The request goes to a local process, not a remote endpoint.
- Durable suppression compares normalized candidate identity, not prose intent or
  patterns. Identical queued category candidates are also not duplicated. A
  changed proposal can still appear, as the ticket's known limit states.
- Section D precedes C to keep the route commit buildable. Every letter still
  has one commit per affected repository, with backend pushed before UI.
- No pin moves means no dependency pins; section K explicitly requires the two
  devshell inventory entries. Branch changes apply only to changed repositories.
- Existing navigation test order had to gain Review. Its count remains three;
  the six new UI tests alone raise the full suite from 47 to 53.
- Setup's Colours and advisory rows share the initial Settings response; separate
  explicit saves/reverts reload their configuration through the existing API.
- Calendar colour still follows the Static/Dynamic partition. No Calendar,
  Routines, Tasks or Priorities screen source changed. No approver, override-clear,
  recurrence import, model management, extra producers or scheduling was added.
- Final self-referential SHA reporting, local acceptance exclusions, runtime file
  scope and the explicit contract-check loopback exception are qualified above.

## Known limits, verbatim

1. **One producer.** `Advise`, `Clarify` and `Batch` have no mainline equivalent.
2. **Runs are manual.** Nothing schedules or batches them.
3. **Only Task ids and titles are sent.** A richer payload would need its own decision about what may leave the machine.
4. **The endpoint must be loopback.** A remote model is a different privacy question and is not opened here.
5. **No model management.** Whether the configured model exists is discovered when a run fails.
6. **Rejection suppresses by candidate, not by pattern.** A differently-worded proposal for the same Task can still arrive.
7. **An uncaptured recurring commitment still occupies no planning capacity**, and the operator's blocking-event workaround is temporary.
8. **The Calendar export still records no approver**, and the Routines screen still cannot clear an override.
