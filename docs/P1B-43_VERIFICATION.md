# P1B-43 verification

## What the agent did not verify

**No test contacted Google, and nothing here shows an event reaching a
calendar.** The orchestrator tests apply to the recording client. The Routines
screen was exercised in Vitest with the Tauri HTTP plugin mocked, and its exact
request bodies were replayed against a real local orchestrator. It has not been
seen in a webview. The operator acceptance steps in the ticket are
outstanding.

## Four things the ticket did not anticipate

Each is inside a repository the ticket changes. None needed another
repository to change. They are stated first because each one differs from
what the ticket says.

1. **An existing test asserted the opposite of section A.** P1B-29's judgment
   call 7 required `automation_worker` from the caller, and its test 4,
   `user_authority_is_rejected_without_client_calls_and_logged`, asserted that
   a `user` approve is rejected with no client call. Section A makes that test
   fail. It was rewritten in commit A as
   `every_approver_authority_exports_as_the_automation_worker`, which asserts
   the new behaviour for all six authority values the API accepts. The count
   of existing tests stays 348, and five are added.
2. **A Calendar operation result carries no authority, so test 2 cannot
   assert one.** The ticket says the GitHub path "records `AutomationWorker`
   on every applied operation result" and asks test 2 to show the Calendar
   results "report `AutomationWorker`". The GitHub result body has an
   `authority_source` per operation. The Calendar result body has
   `operation_id`, `status` and `message`, and its stored form is
   `ubu_core::projection::OperationResult`, which is in a read-only
   repository. Adding the field would change the Calendar API contract and
   the OpenAPI document, against judgment calls 2 and 4. **The field was not
   added.** Test 2 asserts what is observable: see below.
3. **The override route is `PUT`, not `POST`.** The ticket says
   `POST /routine/:objective_id/override/:local_date`. The orchestrator serves
   `PUT` and `DELETE` on that path, and answers a `POST` with 405. The client
   sends `PUT`. Confirmed against the real orchestrator.
4. **A routine must not carry a priority.** The ticket lists priority among
   the editor's fields. `ubu-core` refuses it: `routine Objective must not
   carry priority`, as `objective_invalid`. The form does not offer priority
   and says why.

## Pushed revisions, in landing order

Baseline: `ubu-orchestrator` `bf3b658`, `ubu-ui` `109b110`, `ubu-devshell`
`32051d1`, each on `main` with a clean tree, as was every sibling repository.
Work is on `p1b-43-export-and-routines` in the three that change.

| Order | Repository | Sections | Pushed revision |
| --- | --- | --- | --- |
| 1 | `ubu-orchestrator` | A, B | `8150d303d280f446e2f7fec4012d068c3e82896f` |
| 2 | `ubu-ui` | C–F | `c94b6acbf307461e3380551e1f0949df6e245d23` |
| 3 | `ubu-orchestrator` | F | the commit that carries this file |
| 4 | `ubu-devshell` | G | recorded in `ubu-devshell/docs/P1B-43_PINS.md` |

A file cannot contain the hash of the commit that contains it. The final
`ubu-orchestrator` and `ubu-devshell` revisions and the
`scripts/show-revs.sh` output are recorded in
`ubu-devshell/docs/P1B-43_PINS.md`, as in P1B-39 and P1B-40.

| Section | Repository | Commit | Change |
| --- | --- | --- | --- |
| A | `ubu-orchestrator` | `291303c` | The gate call, the permit check extracted, and the contradicted P1B-29 test rewritten. |
| B | `ubu-orchestrator` | `8150d30` | Five tests. |
| C | `ubu-ui` | `50f8c2c` | Six path constants, three schema versions, six client wrappers. |
| D | `ubu-ui` | `bb02b30` | `src/routes/Routines.tsx`, `src/components/RoutineFields.tsx`, the navigation entry. |
| E | `ubu-ui` | `8cb8cc9` | Eight tests. |
| F | `ubu-ui` | `c94b6ac` | `docs/ROUTINES.md`, `docs/NAVIGATION.md`, two stale lines. |
| F | `ubu-orchestrator` | This commit | `docs/CALENDAR_APPLY_AUTHORITY.md`, two stale passages, this record. |
| G | `ubu-devshell` | see `P1B-43_PINS.md` | The two pins. |

**No dependency pin moved.** No `Cargo.toml`, `Cargo.lock`, `package.json` or
`package-lock.json` changed in any repository. The pins that change are
`ubu_orchestrator` and `ubu_ui` in `ubu-devshell/pinned-revs.toml`, which
section G requires.

`ubu-core` `c77c0a2`, `ubu-store` `7b24cd8`, `ubu-schemas` `4974166`,
`ubu-planning-kernel` `84b6d0d`, `ubu-github-adapter` `4c7e3b6`, `quick-ubu`
`9ccc8b8` and `ubu-design` `f7c4a1d` are at their initial heads with clean
trees. The OpenAPI document did not change: no route, request or response
changed.

## Gates

| Repository | Tests before | Tests after |
| --- | --- | --- |
| `ubu-orchestrator` | **348** | **353** |
| `ubu-ui` | **35** | **43** |
| `ubu-devshell` | none | none; `show-revs.sh` and `check-ui-contract.sh` are the checks |

Green at every commit: orchestrator A 348, B 353; UI C 35, D 35, E 43, F 43.
`npx tsc --noEmit` is clean at every UI commit. `npx vite build` succeeds.

### Clippy

| | Emitted warnings | Unique warnings |
| --- | --- | --- |
| Before, `bf3b658` | 16 | 9 |
| After, `8150d30` | 16 | 9 |

Delta: **0**. The nine unique warnings are the same nine.

Method: `cargo clippy --locked --offline --all-targets --message-format=json`,
after touching `src/lib.rs` so the crate is recompiled. "Emitted" counts JSON
lines with level `warning`. "Unique" deduplicates compiler messages on lint
code, message, file, line and column, because `--all-targets` reports a
warning once for each target that compiles the file. This is P1B-42's method
and script.

`cargo fmt --check` reports differences in files this ticket did not touch. It
was not a gate in earlier tickets and no file was reformatted.

### Lock files

All three are byte-identical. SHA-256 at the baseline and at the final
commits:

```text
e7a0ecf2a14949e5d106ffc3d744605225c56c6ca9d049ff5e698790c6641a15  ubu-orchestrator/Cargo.lock
f17aa89a8b2770b7c28e1aa38fe5a9a1b71c938f58a6bd87cb3666fd448d330f  ubu-ui/package-lock.json
4225e62a5930e8d9347d34b2454eaccf39d2738a7a129cba3f2c5bdfce3d5f56  ubu-ui/src-tauri/Cargo.lock
```

## Section A: the authority parameter

**The `authority` parameter is no longer used in `calendar_apply.rs`.** It had
one use, the gate call, which now passes
`ubu_core::AuthoritySource::AutomationWorker`. The parameter stays in the
signature of `calendar_apply::approve`, renamed `_approver_authority` so the
compiler does not warn, with a comment saying what it is.

**The API field was not removed.** `CalendarProjectionApproveRequest` still
requires `authority_source`, `api/calendar_projection.rs` still converts it
and passes it, and every value accepted before is accepted now.

The permit check was moved, unchanged in substance, from inline in `approve`
to `pub fn ensure_permit_matches`, so test 5 can reach it. `approve` calls it
at the same point.

## Test 1: the operations the recording client received

`user_approval_applies_every_operation_to_the_client`. The request carried
`"authority_source":"user"`. The response was 200 with status `applied`. The
recording client received, verbatim:

```json
[
  {
    "event": {
      "color_id": "3",
      "end_at": "2026-09-25T09:15:00Z",
      "external_id": "01a0e86145fa79338903f77b6912abb3",
      "reminders_minutes": [],
      "start_at": "2026-09-25T09:00:00Z",
      "summary": "Breakfast",
      "task_id": "task_01a0e86145fa79338903f77b6912abb3",
      "transparent": false
    },
    "kind": "insert_event"
  },
  {
    "event": {
      "color_id": "9",
      "end_at": "2026-09-25T10:15:00Z",
      "external_id": "01a0e86145fc79d189c6d455715e34a6",
      "reminders_minutes": [],
      "start_at": "2026-09-25T10:00:00Z",
      "summary": "Work Time",
      "task_id": "task_01a0e86145fc79d189c6d455715e34a6",
      "transparent": true
    },
    "kind": "insert_event"
  },
  {
    "event": {
      "color_id": "6",
      "end_at": "2026-09-25T11:15:00Z",
      "external_id": "01a0e86145fe76f3bbd8ec452d7bea17",
      "reminders_minutes": [],
      "start_at": "2026-09-25T11:00:00Z",
      "summary": "Standup",
      "task_id": "task_01a0e86145fe76f3bbd8ec452d7bea17",
      "transparent": false
    },
    "kind": "insert_event"
  }
]
```

The test asserts these equal the preview's three operations in order, and
that the next preview has no operations. Before section A the client received
nothing and the status was `failed`.

## Test 2: the operation results

`user_approved_results_match_what_the_github_path_reports`. Calendar results
for a `user` approve, verbatim:

```json
[
  {
    "message": null,
    "operation_id": "calendar-create-01a0e8614648731197866c0b3feba9ec",
    "status": "applied"
  },
  {
    "message": null,
    "operation_id": "calendar-create-01a0e861464a73c0accdd2d1a34a4a63",
    "status": "applied"
  },
  {
    "message": null,
    "operation_id": "calendar-create-01a0e861464b7ef0ae67a00891ba4d26",
    "status": "applied"
  }
]
```

GitHub results for a `user` approve in the same process, verbatim:

```json
[
  {
    "authority_source": "automation_worker",
    "message": "managed labels applied: ubu-managed",
    "operation_id": "label-apply-ubu-project-ubu-orchestrator-7-ubu-managed",
    "status": "applied"
  }
]
```

**The Calendar results do not report an authority**, for the reason given at
the top. The test asserts that every Calendar operation is `applied`, that
nothing in a Calendar result attributes the export to `user`, that the GitHub
result reports `automation_worker`, and that all four boundary Log entries
written by the two approvals record `automation_worker`. The parity the
ticket asks for is shown in the Log, not in the result body.

## Test 3: the boundary Log

`user_approval_is_logged_at_the_boundary_as_the_automation_worker`. One entry
per operation. Each entry's `object_refs` are the preview id and the id of
the operation the result reports as `applied`. Verbatim:

```json
[
  {
    "object_refs": [
      "proj_01a0e861463b73729572223b04f5b6e1",
      "calendar-create-01a0e86146247573934494050406605c"
    ],
    "payload": {
      "actor_identity_ref": {
        "id": "identity_01a0e86146197ba2b9016f79c0ca0c0e",
        "object_type": "Identity"
      },
      "adjudication_result": "accepted",
      "authority_source": "automation_worker",
      "compartment_ref": {
        "id": "comp_01a0e861463e7040bced96b95b8fbe37",
        "object_type": "Compartment"
      },
      "effective_time": "2026-09-25T08:00:00Z",
      "member_evaluated": "no_external_export",
      "provenance": {
        "authority_source": "automation_worker",
        "created_at": "2026-09-25T08:00:00Z",
        "source": {
          "source_id": "01a0e86146247573934494050406605c",
          "source_kind": "google_calendar"
        }
      },
      "reason": "Resolved Compartment policy explicitly permits external export for projection operation calendar-create-01a0e86146247573934494050406605c."
    },
    "provenance": {
      "authority_source": "automation_worker",
      "created_at": "2026-09-25T08:00:00Z",
      "source": {
        "source_id": "01a0e86146247573934494050406605c",
        "source_kind": "google_calendar"
      }
    }
  },
  {
    "object_refs": [
      "proj_01a0e861463b73729572223b04f5b6e1",
      "calendar-create-01a0e861462671e1aaebd7c75046edcc"
    ],
    "payload": {
      "actor_identity_ref": {
        "id": "identity_01a0e86146197ba2b9016f79c0ca0c0e",
        "object_type": "Identity"
      },
      "adjudication_result": "accepted",
      "authority_source": "automation_worker",
      "compartment_ref": {
        "id": "comp_01a0e861463e7040bced96e6afa2b663",
        "object_type": "Compartment"
      },
      "effective_time": "2026-09-25T08:00:00Z",
      "member_evaluated": "no_external_export",
      "provenance": {
        "authority_source": "automation_worker",
        "created_at": "2026-09-25T08:00:00Z",
        "source": {
          "source_id": "01a0e861462671e1aaebd7c75046edcc",
          "source_kind": "google_calendar"
        }
      },
      "reason": "Resolved Compartment policy explicitly permits external export for projection operation calendar-create-01a0e861462671e1aaebd7c75046edcc."
    },
    "provenance": {
      "authority_source": "automation_worker",
      "created_at": "2026-09-25T08:00:00Z",
      "source": {
        "source_id": "01a0e861462671e1aaebd7c75046edcc",
        "source_kind": "google_calendar"
      }
    }
  },
  {
    "object_refs": [
      "proj_01a0e861463b73729572223b04f5b6e1",
      "calendar-create-01a0e86146287541b74064bbebc556ae"
    ],
    "payload": {
      "actor_identity_ref": {
        "id": "identity_01a0e86146197ba2b9016f79c0ca0c0e",
        "object_type": "Identity"
      },
      "adjudication_result": "accepted",
      "authority_source": "automation_worker",
      "compartment_ref": {
        "id": "comp_01a0e861463f7f31bf0a19d2e4f865c8",
        "object_type": "Compartment"
      },
      "effective_time": "2026-09-25T08:00:00Z",
      "member_evaluated": "no_external_export",
      "provenance": {
        "authority_source": "automation_worker",
        "created_at": "2026-09-25T08:00:00Z",
        "source": {
          "source_id": "01a0e86146287541b74064bbebc556ae",
          "source_kind": "google_calendar"
        }
      },
      "reason": "Resolved Compartment policy explicitly permits external export for projection operation calendar-create-01a0e86146287541b74064bbebc556ae."
    },
    "provenance": {
      "authority_source": "automation_worker",
      "created_at": "2026-09-25T08:00:00Z",
      "source": {
        "source_id": "01a0e86146287541b74064bbebc556ae",
        "source_kind": "google_calendar"
      }
    }
  }
]
```

`automation_worker` appears in the payload, in the payload's provenance and
in the Log entry's provenance. `user` appears nowhere.

## Tests 4 and 5

4. `no_external_export_still_refuses_a_user_approval`: status `failed`, no
   client call, three `calendar_export_rejected` diagnostics, three boundary
   entries with `member_evaluated` `no_external_export`. The reason names the
   policy and not the approver's authority.
5. `a_permit_for_another_operation_is_an_internal_error`: a real permit from
   the core gate for one operation is checked against another.
   `ensure_permit_matches` returns `AppError::Internal("Calendar export permit
   does not match the operation")`, which is HTTP 500. The permit's own
   operation passes.

   **Only the operation-id half of the check can be made to fail.** The core
   gate issues a permit to the automation worker only, and `ExportPermit` has
   no public constructor, so a permit with another authority cannot be built
   outside `ubu-core`.

## Test 37: the posted body

Verbatim:

```json
{
  "schema_version": "ubu.orchestrator.objective.v1",
  "mode": "evergreen",
  "title": "Synthetic morning review",
  "description": "Synthetic description",
  "recurrence": {
    "timezone": "UTC",
    "rule": {
      "kind": "weekly",
      "weekdays": [
        "mon",
        "thu"
      ]
    }
  },
  "routine_instance_template": {
    "title": "Synthetic morning review",
    "duration_estimate": {
      "type": "fixed",
      "seconds": 1800
    },
    "nominal_start": "09:00:00",
    "placement": "static",
    "occupies_capacity": true,
    "tags": [
      "personal"
    ],
    "reminder_minutes": [
      10,
      0
    ],
    "category_tag": "personal"
  }
}
```

`mode` is `evergreen`. The rule is `weekly` with the two chosen weekdays, in
week order although Thursday was chosen first. The template has the chosen
placement. No `priority` is sent.

## Test 40: the overlap rendering

Verbatim, the text of each element inside the one `role="alert"`, in order:

```text
STRONG: Nothing was written
SPAN: 1 routine overlap; nothing was written. Routines must not overlap: stagger the start time, shorten the routine, or set occupies_capacity to false.
STRONG: Routine overlap (objective_routine_overlap)
SPAN: Conflict with another routine: routine `obj_synthetic_standup` (Synthetic stand-up) 09:15:00-09:45:00 would overlap routine `obj_synthetic_review` (Synthetic morning review) 09:00:00-09:30:00, first on 2026-09-30 (up to 105 dates in the next year)
DT: This routine
DD: Synthetic stand-up, 09:15:00-09:45:00 (obj_synthetic_standup)
DT: Conflicts with
DD: Synthetic morning review, 09:00:00-09:30:00 (obj_synthetic_review)
DT: First colliding date
DD: 2026-09-30
DT: Extent
DD: up to 105 dates in the next year
```

The second and fourth lines are the orchestrator's summary and message,
unchanged. The four pairs beneath are read from the message by the screen.
The test asserts that the list is not reloaded and that the operator's
entries are kept.

The message in the fixture is the orchestrator's own, from a real run, with
synthetic ids substituted:

```text
Conflict with another routine: routine `obj_01a0e862a8827f43ad3cb80aec6803d1` (Synthetic stand-up) 09:15:00-09:45:00 would overlap routine `obj_01a0e862a87873b08c2ff3565235cb8c` (Synthetic morning review) 09:00:00-09:30:00, first on 2026-09-30 (up to 105 dates in the next year)
```

## The UI's bodies against a real orchestrator

The bodies the UI tests assert were replayed against the real binary on a
loopback port with a temporary store. Responses are cut at 260 characters.

```text
POST /objective 201 {"schema_version":"ubu.orchestrator.objective.v1","objective_id":"obj_01a0e8685ba070829551768320897e96","version":1}
POST /objective 201 {"schema_version":"ubu.orchestrator.objective.v1","objective_id":"obj_01a0e8685ba77bb3b10b5d8f7b7a69a5","version":1}
GET /objective/obj_01a0e8685ba070829551768320897e96 200 {"schema_version":"ubu.orchestrator.objective.v1","objective_id":"obj_01a0e8685ba070829551768320897e96","version":1,"is_routine":true,"payload":{"description":"Synthetic description","id":"obj_01a0e8685ba
PATCH /objective/obj_01a0e8685ba070829551768320897e96 200 {"schema_version":"ubu.orchestrator.objective.v1","objective_id":"obj_01a0e8685ba070829551768320897e96","version":2,"notice":"The routine template changed. Occurrences already materialized keep the temp
PATCH /objective/obj_01a0e8685ba070829551768320897e96 200 {"schema_version":"ubu.orchestrator.objective.v1","objective_id":"obj_01a0e8685ba070829551768320897e96","version":3}
PUT /routine/obj_01a0e8685ba070829551768320897e96/override/2026-10-01 200 {"schema_version":"ubu.orchestrator.routine_override.v1","objective_id":"obj_01a0e8685ba070829551768320897e96","local_date":"2026-10-01","overridden":true,"diagnostics":[]}
PATCH /objective/obj_01a0e8685ba070829551768320897e96 200 {"schema_version":"ubu.orchestrator.objective.v1","objective_id":"obj_01a0e8685ba070829551768320897e96","version":5}
```

Both creates, a template edit, a recurrence edit that clears the description,
the override by `PUT`, and the change of status to `abandoned` were accepted.

## No Google, no credential, no real data

- **No test contacted Google.** The orchestrator suite ran under a seccomp
  filter that refuses every `AF_INET` and `AF_INET6` socket, at both commits,
  and passed. The Calendar tests use `export_mode: "mock"` and the recording
  client. The UI suite ran under a Node preload that refuses every socket
  connect, DNS lookup and real `fetch`; it recorded 0 attempts at every
  commit.
- **No test read a credential.** The orchestrator suite ran with
  `GITHUB_TOKEN`, `UBU_GOOGLE_CREDENTIALS_PATH` and
  `UBU_GOOGLE_TOKEN_CACHE_PATH` removed from its environment. The probe and
  the contract check start the orchestrator with an emptied environment, a
  temporary `HOME` and a temporary store.
- **No real data.** Every routine, Task and event title in a fixture is
  synthetic: "Synthetic morning review", "Synthetic stand-up", "Synthetic
  stretch", "Synthetic ledger check", and P1B-29's existing "Breakfast",
  "Work Time" and "Standup". No Quick UbU export, calendar or routine of the
  operator's was read.
- **No UI test calls the global `fetch`.** `tests/setup.ts` fails any test
  that does. `tests/routines.test.tsx` throws on any request it does not
  expect.

## What was read and written, and where

The work read and wrote files under `~/ubu-phase1b` and under `/tmp` only.
The Google OAuth token location in the home directory was not read, written,
moved or listed, and the home directory was not listed.

Three things outside those two trees were touched, none of them by the
ticket's own steps:

- the toolchains read their own installations and caches in the home
  directory: `cargo` and `rustc`, `node` and `npx`;
- `git` read its configuration and signing key to sign commits, and the SSH
  key to push;
- the agent's session notes are kept under `~/.claude`.

## The contract check

Before, at the baseline. Exit status 0.

```text
build: cargo build --locked --offline in /home/sean/ubu-phase1b/ubu-orchestrator
   Compiling ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.68s
built: /home/sean/ubu-phase1b/ubu-orchestrator/target/debug/ubu_orchestrator
start: orchestrator on ephemeral port 41401, store in /tmp/ubu-contract-check.hhdQ5J
defaults:
  ubu-ui            DEFAULT_ORCHESTRATOR_PORT = 7878
  ubu-orchestrator  UBU_ORCHESTRATOR_PORT unwrap_or = 7878
  the two defaults agree
requests against http://127.0.0.1:41401:
  200 GET http://127.0.0.1:41401/health
  201 POST http://127.0.0.1:41401/task
  200 GET http://127.0.0.1:41401/tasks?schema_version=ubu.orchestrator.task_read.v1&status=active
  200 PATCH http://127.0.0.1:41401/task/task_01a0e85a177e7fa3abab46ff63c6b035
  200 POST http://127.0.0.1:41401/planning/generate
  200 GET http://127.0.0.1:41401/next-action?schema_version=ubu.orchestrator.next_action.v1
paths in the live /openapi.json:
  200 GET http://127.0.0.1:41401/openapi.json
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
stopped: orchestrator pid 294197
removed: /tmp/ubu-contract-check.hhdQ5J
```

After, at the final `ubu-ui` code. Exit status 0.

```text
build: cargo build --locked --offline in /home/sean/ubu-phase1b/ubu-orchestrator
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.15s
built: /home/sean/ubu-phase1b/ubu-orchestrator/target/debug/ubu_orchestrator
start: orchestrator on ephemeral port 42015, store in /tmp/ubu-contract-check.PPI2mI
defaults:
  ubu-ui            DEFAULT_ORCHESTRATOR_PORT = 7878
  ubu-orchestrator  UBU_ORCHESTRATOR_PORT unwrap_or = 7878
  the two defaults agree
requests against http://127.0.0.1:42015:
  200 GET http://127.0.0.1:42015/health
  201 POST http://127.0.0.1:42015/task
  200 GET http://127.0.0.1:42015/tasks?schema_version=ubu.orchestrator.task_read.v1&status=active
  200 PATCH http://127.0.0.1:42015/task/task_01a0e869e26f7c02b1f2373371efb282
  200 POST http://127.0.0.1:42015/planning/generate
  200 GET http://127.0.0.1:42015/next-action?schema_version=ubu.orchestrator.next_action.v1
paths in the live /openapi.json:
  200 GET http://127.0.0.1:42015/openapi.json
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
stopped: orchestrator pid 325324
removed: /tmp/ubu-contract-check.PPI2mI
```

**27 path constants before, 33 after: six more.** They are the six section C
adds. Two of them, `OBJECTIVE_READ_PATH` and `OBJECTIVE_EDIT_PATH`, hold the
same path, because `GET` and `PATCH` are two routes on one path. The check
counts constants. `SETTING_PUT_PATH` and `SETTING_DELETE_PATH` already did
the same. The count of distinct paths rose by five.

The check's six requests do not include a routine. Its step 4 proves the six
new paths exist.

## `scripts/show-revs.sh`

Recorded in `ubu-devshell/docs/P1B-43_PINS.md`, with the final revisions and
the contract check run again after the pins.

## Commit trailers

No trailer in `ubu-orchestrator`. `Co-Authored-By: Claude Fable 5.1
<noreply@anthropic.com>` on every P1B-43 commit in `ubu-ui` and
`ubu-devshell`. All commits are signed. Nothing was force-pushed, and no
commit was rewritten.

## Judgment calls

Thirteen of the fifteen were followed without disagreement.

**Call 3 is followed, and the agent disagrees with one effect of it.** The
boundary Log used to record the requested authority. It now records the
worker, as the GitHub path does. The GitHub path also stores the approver, in
its approval record. The Calendar path has no approval record, so after this
ticket **nothing records who approved a Calendar export**. Recording the
approver needs a new stored record, which call 4 rules out here. It is a
small later ticket.

**Call 1 is followed, with an observation.** The core gate's authority check
is now constant on both paths. P1B-29 called rejecting user authority
"correct rather than inconvenient". What still refuses an export is the
resolved policy, the conflict check against the applied set, and live-mode
enablement. The loopback port has no per-run token, so any local process can
approve a preview. That was already true of the GitHub path.

## Ambiguities, and the reading taken

1. **"No pin moves" and section G.** Read as in P1B-40: no dependency pin
   moves, and the devshell record is updated last.
2. **Test 2**, as above. The most literal reading that does not change the
   contract.
3. **The rewritten P1B-29 test** is counted as an existing test, so the
   totals are the ticket's.
4. **"Priority" in the editor**, as above. Not offered.
5. **"Status" in the editor** is offered when editing only. `POST /objective`
   rejects unknown fields and has no `status`; a new Objective is `active`.
6. **"`POST`" for the override**, as above. `PUT`.
7. **"The version the list returned."** The screen reads each routine after
   the list, and uses the version from that read. It is the same number
   unless the routine changed between the two reads, in which case it is the
   newer.
8. **The orchestrator replaces a whole template or recurrence on an edit.**
   The screen sends back what it does not show exactly as stored: `effects`,
   `preconditions`, `after`, the enabled range, excluded dates and overrides.
   An edit sends only the parts that changed.
9. **Override times** are entered in this computer's timezone and sent as
   instants.
10. **The category is always one of the tags**, as the importer writes it.
11. **Two stale documents were corrected** beyond the one the ticket names:
    `CALENDAR_APPLY.md` said the caller's authority is not replaced, and
    `CALENDAR_BOOTSTRAP.md` described the blocker as current.
12. **The existing navigation test** was updated from seven screens to
    eight. It is not counted as new.
13. **Judgment call 1 cites line 208** and the preconditions cite line 204.
    The gate call begins at 204 and its authority argument is on 208.

## Known limits

1. **`effects` and `preconditions` are read-only** on the routine editor. Authoring them is a later ticket.
2. **No Review screen**, and nothing produces advisory candidates yet. That is the last large gap.
3. **Routines cannot be deleted**, only abandoned.
4. **Only five recurrence kinds**, matching what the importer emits and the materializer understands.
5. **A template edit does not reach today's occurrence.**
6. **The request's `authority_source` is accepted and not used at the gate**, in both projection paths.
7. **Captured calendar Tasks are still Static and one-off.** Recognising that a run of captured Tasks is really a routine is not attempted.
8. **No Reports and no Log review**, whose stubs P1B-40 deleted.

Found during the work, beyond the eight:

9. **The approver of a Calendar export is not recorded anywhere.**
10. **A Calendar operation result carries no authority.**
11. **An override cannot be cleared from the screen.** The orchestrator has the `DELETE`; the screen does not call it.
12. **A routine's enabled range and excluded dates are kept and not editable.**
13. **The list makes one read per routine.**
14. **Enabling, status changes and the 409 path of the routine editor have no kept test.** The ticket fixes the count at 43.
