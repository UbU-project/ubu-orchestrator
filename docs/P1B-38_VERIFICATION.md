# P1B-38 verification

Baseline: `ubu-orchestrator` `6c84778cae6671a5355dbcfc3415ac18ffcacda9`,
`ubu-devshell` `0499179b860dca24de24c7e37929f4b4d8ff7f17`, `ubu-ui`
`7c1007eb2131dae7dd4997b8054a924974eb5086`. Every sibling repository had a
clean working tree before the first edit. Work is on
`p1b-38-authoring-and-surface` in the three repositories that change.

## Pushed revisions, in landing order

| Order | Repository | Sections | Pushed revision |
| --- | --- | --- | --- |
| 1 | `ubu-orchestrator` | A–D | `df8f4006c09616b3dfadff18f4d177a85bfed3b6` |
| 2 | `ubu-devshell` | E | `dde5ef1e76b4bb71d9f2c7c1551c216fec8e0425` |
| 3 | `ubu-ui` | F–H, and its half of I | `6d6028c423e6b0f9bc710629514a2ff4d3ff85f4` |

This document and `OBJECTIVE_AUTHORING.md` are the orchestrator's half of
section I. They land in one further documentation-only commit on the same
branch, after the three revisions above, and that commit cannot name itself.

| Section | Repository | Commit | Change |
| --- | --- | --- | --- |
| A | `ubu-orchestrator` | `e331fa1` | `GET /task/:task_id` and `GET /tasks`. |
| B | `ubu-orchestrator` | `b6e5780` | `objective_authoring.rs` and the four Objective routes. |
| C | `ubu-orchestrator` | `5885071` | Regenerated OpenAPI document. |
| D | `ubu-orchestrator` | `df8f400` | Twelve tests in two new files. |
| E | `ubu-devshell` | `dde5ef1` | All seven pins and their comments. |
| F | `ubu-ui` | `c8d8002` | Both generated artifacts, four client methods. |
| G | `ubu-ui` | `2c98b43` | `Tasks.tsx`, `TaskFields.tsx`, one navigation entry. |
| H | `ubu-ui` | `b3534a3` | Five tests in `tests/tasks.test.tsx`. |
| I | `ubu-ui` | `6d6028c` | `docs/HANDOFF.md`. |
| I | `ubu-orchestrator` | This commit | `OBJECTIVE_AUTHORING.md` and this record. |

**No pin moved.** `Cargo.toml` is unchanged; the dependency revisions are still
core `c77c0a2d1c3e206b8c18c023bb8b4ee0d06eb2d0`, store
`4066b8184403799fee031eb0660ee1bc12b3c1e8`, adapter
`4c7e3b6d31008a3bd64f28c303b8aa01bca3e2a0` and both planning packages at
`84b6d0d9b621ca9a9df10034a8f8dc660baaca5d`.

**No other repository changed.** After the last push, `ubu-core` `c77c0a2`,
`ubu-schemas` `4974166`, `ubu-store` `7b24cd8`, `ubu-planning-kernel`
`84b6d0d`, `ubu-github-adapter` `4c7e3b6`, `quick-ubu` `9ccc8b8`,
`ubu-design` `f7c4a1d`, `ubu-brand` `faf2005` and `model-committee` `4359c55`
are on `main` at their initial heads with clean trees. `npm ci` and
`npm run generate:typescript` ran in `ubu-schemas`; their output is gitignored
there and the tree stayed clean.

## Gates, per repository

### `ubu-orchestrator`

- Tests: **319 → 331**, zero failed, zero ignored. The full suite ran at every
  commit: 319 at A, B and C, 331 at D. Exactly twelve tests are new, eight in
  `tests/objective_authoring.rs` and four in `tests/task_read.rs`. No existing
  test was changed or removed.
- Clippy: **9 → 9** distinct warnings, the same nine. Counting method:
  `cargo clippy --locked --offline --all-targets --message-format=json`,
  keeping `compiler-message` records at level `warning` that have a span, and
  deduplicating by lint code, message, and the primary span's file, line and
  column, so repeated library and library-test emissions count once. Both
  measurements used that command and that script on this checkout, the first
  at `6c84778` before any edit and the second at `df8f400`. They were taken
  about an hour apart in the same working session rather than in one
  invocation.
- `rustfmt --check --edition 2021 --config skip_children=true` passes for
  every Rust file this ticket created or edited. Formatting elsewhere was left
  alone.
- `Cargo.lock` is byte-identical to the baseline, SHA-256
  `e7a0ecf2a14949e5d106ffc3d744605225c56c6ca9d049ff5e698790c6641a15`.
- The final suite was also run under a seccomp filter that denies `AF_INET`
  and `AF_INET6` socket creation, inherited by every subprocess: 331 passed.
  Fixtures and stores are synthetic and in memory.

### `ubu-devshell`

- One file changed, `pinned-revs.toml`. The repository has no test suite that
  this ticket ran; `scripts/show-revs.sh` is the check, and its output is
  below.
- No lockfile exists in this repository.

### `ubu-ui`

- Tests: **6 → 11**. 6 at F and at G, 11 at H and I. Five tests are new, all in
  `tests/tasks.test.tsx`. `tests/smoke.test.tsx` is unchanged.
- `npx tsc --noEmit` is clean at every commit.
- This repository has no Rust and no clippy; there is no lint script in
  `package.json`, so there is no lint delta to report.
- `package-lock.json` is byte-identical to the baseline, SHA-256
  `4aed9c3e42c125b91b37a507ca07f5b835f5189021b80afe14e0f5bf9e95ac78`.
  `package.json` is unchanged.

## OpenAPI path counts

| | Paths | Operations |
| --- | --- | --- |
| `ubu-orchestrator` before §C (`6c84778`) | 45 | 47 |
| `ubu-orchestrator` after §C (`5885071`) | **49** | **53** |
| `ubu-ui` copy before §F (`7c1007e`) | 23 | |
| `ubu-ui` copy after §F (`c8d8002`) | **49** | **53** |

**The ticket expected 45 → 51. The measured result is 45 → 49.** All six
routes are in the document, as six new operations:

```text
GET   /objective/{objective_id}
GET   /objectives
GET   /task/{task_id}
GET   /tasks
PATCH /objective/{objective_id}
POST  /objective
```

An OpenAPI document keys `paths` by path, not by route. `GET /task/{task_id}`
joins the existing `PATCH` under one key, and the `GET` and `PATCH` on
`/objective/{objective_id}` share another, so six routes add four keys.
Nothing is missing; the expected figure counted routes as paths. §F compiled,
which it could not have done had any route the client names been absent.
`ubu-ui`'s copy is byte-identical to the orchestrator's document.

## The refresh, before any client change

Immediately after both generators ran, with `src/api/client.ts` untouched:

- `npx tsc --noEmit`: clean.
- `npm test`: **6 passing**, in one file.

The refresh alone changed nothing observable, as the ticket predicted. The
gate was then confirmed from the other side: with the path constant misspelt
as `"/taskz"`, `tsc` fails with `TS1360 ... does not satisfy the expected
type`. The misspelling was reverted before the commit.

`generate:types` emitted `index.d.ts` from **83 schemas**, 31,786 bytes against
the previous 20,432.

**Invocation.** The ticket says to run both generators from `ubu-devshell`.
`ubu-devshell` has no `package.json`; the two npm scripts are defined in
`ubu-ui/package.json` and call the devshell scripts by bare name. They were
run from `ubu-ui` with the devshell scripts directory on `PATH`:

```sh
PATH="$PWD/../ubu-devshell/scripts:$PATH" npm run generate:types
PATH="$PWD/../ubu-devshell/scripts:$PATH" npm run generate:api
```

No script and no `package.json` was changed, and nothing was copied by hand.
Both scripts found their sources. Each rewrites the destination `README.md`
with the absolute source path it used, so those two files now name this
machine's checkout directory, as the previous copies did.

## `scripts/show-revs.sh` after §E

```text
Recorded R_* baseline: post-O20 R_orchestrator, post-GA2 R_adapter, post-S17 R_schemas, post-C12 R_core, post-ST7 R_store

REPO                     BRANCH         HEAD      SIG                 TREE   PINNED    STATUS
----                     ------         ----      ---                 ----   ------    ------
ubu_design               main           f7c4a1db  signed-ok           clean  (unset)   unset
ubu_schemas              main           4974166a  signed-ok           clean  4974166a  OK
ubu_core                 main           c77c0a2d  signed-ok           clean  c77c0a2d  OK
ubu_store                main           7b24cd82  signed-ok           clean  7b24cd82  OK
ubu_github_adapter       main           4c7e3b6d  signed-ok           clean  4c7e3b6d  OK
ubu_planning_kernel      main           84b6d0d9  signed-ok           clean  84b6d0d9  OK
ubu_orchestrator         p1b-38-authoring-and-surface df8f4006  signed-ok           clean  df8f4006  OK
ubu_ui                   main           7c1007eb  signed-ok           clean  7c1007eb  OK
ubu_brand                main           faf2005a  signed-ok           clean  (unset)   unset
```

Exit status 0. All seven pinned repositories report `OK`. `ubu_design` and
`ubu_brand` have never been pinned and remain `unset`.

Three things about this record are worth stating plainly.

- The first line of the output is printed from a string inside
  `show-revs.sh` and still describes the pre-P1B baseline. The ticket changes
  `pinned-revs.toml` only, so the script was left alone.
- `ubu_ui` is pinned to `7c1007e`, which was its current pushed revision when
  §E ran. §F–§H then moved `ubu-ui`. The landing order makes this
  unavoidable: §E must precede §F, and the pin cannot name a revision that
  does not exist yet.
- `ubu_orchestrator` is pinned to §D's revision as instructed. The
  documentation commit that carries this file follows it.

So on these branches `show-revs.sh` now reports `MISMATCH` for `ubu_ui` and
`ubu_orchestrator`. The pins are correct for the functional revisions and one
commit behind on documentation and the surface. A follow-up pin bump, after
the branches merge, would close it.

## Test 2: the admitted routine, verbatim

The stored payload, read from the store after `POST /objective`. The request
supplied `schedule_version: 9` and `template_version: 9`; both were
overwritten.

```json
{
  "id": "obj_01a0e5afb3d472c2975afd2e85155ed3",
  "mode": "evergreen",
  "provenance": {
    "authority_source": "user",
    "created_at": "2026-09-28T09:00:00Z"
  },
  "recurrence": {
    "rule": {
      "kind": "daily"
    },
    "schedule_version": 1,
    "timezone": "America/New_York"
  },
  "routine_instance_template": {
    "category_tag": "health",
    "duration_estimate": {
      "seconds": 300,
      "type": "fixed"
    },
    "nominal_start": "12:00:00",
    "placement": "static",
    "tags": [
      "health"
    ],
    "template_version": 1,
    "title": "Synthetic stretch"
  },
  "status": "active",
  "title": "Synthetic stretch"
}
```

## Test 3: occurrence payloads, native beside imported

Two stores under the same fixed clock, `2026-09-28T09:00:00Z`. One holds a
routine authored through `POST /objective`; the other holds the same routine
arriving through `POST /import/quick-ubu`. Each was materialized over the same
24-hour horizon and the admitted occurrence rows were read back from the
store. The two generated ids are replaced by `<task>` and `<objective>`;
nothing else is altered.

Native:

```json
[
  {
    "category_tag": "health",
    "duration_estimate": {
      "seconds": 300,
      "type": "fixed"
    },
    "id": "<task>",
    "objective_id": "<objective>",
    "occurrence": {
      "key": "<objective>/s1/2026-09-28T12:00:00/static/t1",
      "local_date": "2026-09-28",
      "routine_objective_id": "<objective>"
    },
    "provenance": {
      "authority_source": "system",
      "created_at": "2026-09-28T09:00:00Z",
      "source": {
        "source_id": "<objective>/s1/2026-09-28T12:00:00/static/t1",
        "source_kind": "routine_instantiation"
      }
    },
    "static_window": {
      "end": "2026-09-28T16:05:00Z",
      "start": "2026-09-28T16:00:00Z"
    },
    "status": "active",
    "tags": [
      "health"
    ],
    "title": "Synthetic stretch"
  }
]
```

Imported:

```json
[
  {
    "category_tag": "health",
    "duration_estimate": {
      "seconds": 300,
      "type": "fixed"
    },
    "id": "<task>",
    "objective_id": "<objective>",
    "occurrence": {
      "key": "<objective>/s1/2026-09-28T12:00:00/static/t1",
      "local_date": "2026-09-28",
      "routine_objective_id": "<objective>"
    },
    "provenance": {
      "authority_source": "system",
      "created_at": "2026-09-28T09:00:00Z",
      "source": {
        "source_id": "<objective>/s1/2026-09-28T12:00:00/static/t1",
        "source_kind": "routine_instantiation"
      }
    },
    "static_window": {
      "end": "2026-09-28T16:05:00Z",
      "start": "2026-09-28T16:00:00Z"
    },
    "status": "active",
    "tags": [
      "health"
    ],
    "title": "Synthetic stretch"
  }
]
```

The test asserts the key shape, the placement (`static_window`, and the
absence of `allowed_time_range`) and `duration_estimate` individually, and
then that the two payloads are equal in full.

## Test 5: the version counters, all three cases

`object_version` is the store row's version. Read from the store after each
edit.

```json
{
  "created": {
    "object_version": 1,
    "schedule_version": 1,
    "template_version": 1
  },
  "edit_changing_neither": {
    "object_version": 4,
    "schedule_version": 2,
    "template_version": 2
  },
  "recurrence_edit": {
    "object_version": 3,
    "schedule_version": 2,
    "template_version": 2
  },
  "template_edit": {
    "object_version": 2,
    "schedule_version": 1,
    "template_version": 2
  }
}
```

| Step | Edit | `schedule_version` | `template_version` |
| --- | --- | --- | --- |
| created | | 1 | 1 |
| template edit | `nominal_start` 12:00 → 13:30 | 1 | **2** |
| recurrence edit | daily → weekly on Tuesday and Friday | **2** | 2 |
| edit changing neither | rename, with both objects restated as stored | 2 | 2 |

Only the template edit's response carried the `notice` that materialized
occurrences are unaffected until the next `materialize`.

## Test 7: a native Objective across an import, verbatim

One plain Objective and one routine, both native, the routine sharing its
title with an imported routine. The fixture snapshot was then imported twice,
five minutes later by the clock. Each entry is the store row's `version`,
`status` and `payload`.

Before:

```json
{
  "objective": {
    "payload": {
      "id": "obj_01a0e5afb3d87ad2aa35405bddf1e8eb",
      "priority": 40,
      "provenance": {
        "authority_source": "user",
        "created_at": "2026-09-28T09:00:00Z"
      },
      "status": "active",
      "title": "Synthetic routine 1"
    },
    "status": "active",
    "version": 1
  },
  "routine": {
    "payload": {
      "id": "obj_01a0e5afb3de7790bfe1f402786f75fb",
      "mode": "evergreen",
      "provenance": {
        "authority_source": "user",
        "created_at": "2026-09-28T09:00:00Z"
      },
      "recurrence": {
        "rule": {
          "kind": "daily"
        },
        "schedule_version": 1,
        "timezone": "America/New_York"
      },
      "routine_instance_template": {
        "category_tag": "health",
        "duration_estimate": {
          "seconds": 300,
          "type": "fixed"
        },
        "nominal_start": "03:00:00",
        "placement": "static",
        "tags": [
          "health"
        ],
        "template_version": 1,
        "title": "Synthetic stretch"
      },
      "status": "active",
      "title": "Synthetic routine 1"
    },
    "status": "active",
    "version": 1
  }
}
```

After:

```json
{
  "objective": {
    "payload": {
      "id": "obj_01a0e5afb3d87ad2aa35405bddf1e8eb",
      "priority": 40,
      "provenance": {
        "authority_source": "user",
        "created_at": "2026-09-28T09:00:00Z"
      },
      "status": "active",
      "title": "Synthetic routine 1"
    },
    "status": "active",
    "version": 1
  },
  "routine": {
    "payload": {
      "id": "obj_01a0e5afb3de7790bfe1f402786f75fb",
      "mode": "evergreen",
      "provenance": {
        "authority_source": "user",
        "created_at": "2026-09-28T09:00:00Z"
      },
      "recurrence": {
        "rule": {
          "kind": "daily"
        },
        "schedule_version": 1,
        "timezone": "America/New_York"
      },
      "routine_instance_template": {
        "category_tag": "health",
        "duration_estimate": {
          "seconds": 300,
          "type": "fixed"
        },
        "nominal_start": "03:00:00",
        "placement": "static",
        "tags": [
          "health"
        ],
        "template_version": 1,
        "title": "Synthetic stretch"
      },
      "status": "active",
      "title": "Synthetic routine 1"
    },
    "status": "active",
    "version": 1
  }
}
```

The two are equal. The first import created five routines; the second left
all five unchanged and reported nothing stale. Seven Objectives exist
afterwards: two native, five imported.

## No `ubu-ui` test reached the network

Established three ways.

1. **By construction.** Every test stubs `fetch` with `vi.stubGlobal`. In
   `tests/tasks.test.tsx` the stub answers every request, throws on any
   request it does not recognise, and asserts that each request's origin is
   the loopback address the client is configured for.
2. **By instrumentation.** The suite was run with a preload, outside the
   repository, that replaces `net.Socket.prototype.connect`, `dns.lookup` and
   the real global `fetch` with functions that record the attempt and throw.
   The preload loaded in 19 Node processes. It recorded **zero** attempts and
   all 11 tests passed.
3. **By denial.** The suite was run under the seccomp filter that denies
   `AF_INET` and `AF_INET6` sockets: 11 passed.

Network was used for `git push`, for `npm ci` in `ubu-schemas` and `ubu-ui`,
and for nothing else. Every Cargo command ran `--locked --offline`.

## Live check

Beyond the ticket's requirements, the built orchestrator was started on a
loopback port against a temporary database, and the exact requests the client
sends were issued with `curl`: capture with `category_tag` and `tags`, the
list with `schema_version` and `status` in the query, the single read, an
edit carrying version 1, and a second edit carrying the now stale version 1.
The last returned HTTP 409 with `version_conflict`, in the shape the Tasks
screen handles. The screen itself was exercised through its tests, not in a
browser or the Tauri shell.

## Judgment calls

There is no disagreement with any of the twelve. Two of them describe
something the code does not quite have, and one has a consequence worth
knowing.

- **8, "they are `moot`".** The Objective status vocabulary is `open`,
  `active`, `satisfied`, `abandoned`. `moot` is a Task status. Following the
  second half of the call — the existing vocabulary is the whole mechanism —
  `PATCH` accepts those four values and withdrawing an Objective is
  `abandoned`. No status was added.
- **12, "a `NAV` array".** The array in `App.tsx` is `navItems`. One entry was
  added to it, and one member to the `RouteId` union.
- **11, pins for every repository.** Done. The consequence is recorded under
  `show-revs.sh` above: the landing order leaves two pins one step behind.

## Found, not fixed

- **Native routines are not checked for overlap.** The ticket specifies three
  rejections and the importer's overlap check is not among them. Measured: two
  native Static routines at the same hour are both admitted with HTTP 201;
  planning reports `routine_occurrences_overlap`; and the next
  `POST /import/quick-ubu` is refused with `1 overlapping routine pair in 1
  group; nothing was imported`. An authoring slip therefore blocks imports
  until it is corrected. This is the item most worth a follow-up.
- **A title of only spaces is admitted** by `POST /objective`, as it is by
  `POST /task`. "Empty or absent" was read literally. The Tasks screen trims
  titles before sending.
- **The orchestrator serves no CORS headers**, before and after this ticket;
  an `OPTIONS` request returns HTTP 405. This predates P1B-38 and affects
  every screen equally. Whether the Tauri shell needs them was not tested.

## Ambiguities, and the literal reading taken

- **`TASK_READ_SCHEMA_VERSION`, "validated the way `user_action.rs` does".**
  A `GET` has no body. `schema_version` is an optional query parameter; a
  wrong value is `unknown_schema_version`, as in `user_action.rs`. Absence is
  accepted, because the ticket's own tests call `GET /tasks` bare. The client
  always sends it.
- **`placement` on a Task.** Tasks have no placement field. It is derived:
  `static` when the Task has a `static_window`, otherwise `planned`, matching
  how an occurrence's template placement is materialized.
- **"ordered by status then creation time then id".** Each call returns one
  status, so the visible order is creation time then id. The query orders by
  all three.
- **Absent optional fields in `GET /tasks`** are omitted rather than `null`,
  following the instruction that `container_id` is omitted for ordinary
  Tasks.
- **Store row status for an Objective.** Not specified. `open` and `active`
  keep the row `active`, which is what keeps a routine live; `satisfied` and
  `abandoned` are written to the row too, which is what makes the importer
  report a withdrawn imported routine as diverged instead of rewriting it.
- **`recurrence` is replaced whole by `PATCH`**, including any `overrides` and
  `exdates` inside it, as the importer does. A client that wants to keep them
  restates them.
- **A category needs a tag.** Core requires `category_tag` to be a member of
  `tags`. The Tasks screen sends the category as a tag on capture. On edit it
  reads the Task's tags with `getTask` and adds the new category to them; it
  never removes a tag.
- **Due date.** The form takes a date. It is sent as the end of that day in
  the operator's local time zone, as an instant.
- **Inline edit is offered for active Tasks only.** The other statuses are
  listed read-only.
- **Checklist headings** show the Container id and step count. The Container's
  name would need `GET /containers`, which is outside the four client methods
  the ticket allows.
- **Commit trailers.** Each commit carries a `Co-Authored-By` line, which
  earlier P1B commits do not.

## Known limits

Recorded verbatim from the ticket, and again in
[OBJECTIVE_AUTHORING.md](OBJECTIVE_AUTHORING.md).

1. **No `target_date` on Objectives.** Quick UbU has one; the schema does not, and adding it is a six-repo chain. Deadlines live on Tasks as `due_at`.
2. **Objectives cannot be deleted**, only mooted. Deliberate, and unlike Preferences.
3. **A template edit does not reach today's occurrence.** It applies at the next `materialize`.
4. **`GET /tasks` does not paginate** and filters only by status. No text search, no category filter, no date range.
5. **The UI covers four routes of fifty-one.** Preferences, decomposition, Containers, routines, the occurrence override, the advisory queue, the Google Calendar chain and Quick UbU import all remain unreachable from the app. Later tickets.
6. **`ubu-ui` still wires the old generic `/projection/*` routes**, not the `/projection/calendar/*` chain from P1B-29…35. Both exist server-side, so nothing is broken, but the app is driving the older projection path.
7. **No routine authoring in the UI.** §B makes it possible over HTTP; the screen comes later.
8. **No Task delete**, unchanged from P1B-27.

Limit 5 as measured: the client gained four methods over three path keys, and
the document holds 49 path keys carrying 53 operations.

Gate logs, the evidence extracts, the clippy output and the two network
harnesses are retained locally in `../.p1b-38-results/`.
