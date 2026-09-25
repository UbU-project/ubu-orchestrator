# P1B-28 verification

## Scope and checks

Implemented sections A–F on local branch `p1b-28-calendar-projection`, starting
from `4dfbb8f1d3519248cc272dd0b05aeeddeac64dd2` with a clean working tree.
Only `ubu-orchestrator` changed. All eleven sibling repositories retained their
initial HEADs and clean trees. `Cargo.toml` and `Cargo.lock` are byte-identical to
the pre-ticket copies: no dependency, pin, lockfile, or path override changed.
No new repository or clone was created. The branch is intentionally not pushed:
the ticket's no-network rule takes precedence over any ordinary push workflow.

No network call, credential read, or Google dependency was added. The event
service has no I/O, clock, or network capability. Preview uses only local store
reads and pure mapping/diff functions; it has no apply or persistence path.
Tests use synthetic data, temporary synthetic import files, in-memory SQLite,
and `Router::oneshot`, without opening HTTP sockets. Cargo ran with
`CARGO_NET_OFFLINE=true`; final test runs used a minimal environment containing
only PATH, HOME and that offline flag. No external service, Google client, OAuth
flow, actual credential, or real Quick UbU dataset was used. No push or fetch was
performed.

`cargo test --all-targets` and `cargo test --doc` passed before every lettered
commit. Section A passed 211 tests; B–E passed 217; F passed **222**, with zero
failures or ignored tests in the final suite. The doc-test target has zero tests.
The total follows the explicit requirements: 211 existing + **six** module unit
tests + **five** integration tests = **222**. The prompt's expected 217 is an
arithmetic discrepancy; no requested test was omitted to match it.

`cargo clippy --all-targets` succeeded before and after in this ticket session:
**10 diagnostic warnings before, 10 after, delta 0**. Counting method: count lines
beginning `warning:`, excluding Cargo's backtick-prefixed package summaries with
`generated N warning(s)`, including duplicate summaries. The complete
warning-message multisets are identical, not merely the totals.

`cargo run --example generate_openapi` ran offline and regenerated the committed
artifact. The new GET route and all three response/event/operation schemas are
present. The response schema documents that preview stores nothing and operations
are all creates until P1B-29. `git diff --check` passes. Only new Rust files were
formatted with `rustfmt --edition 2021`; no repository-wide formatting was run.

## Transparency regression: integration test 2

The synthetic imported Work Time routine is Dynamic, mandatory and transparent.
The original code was first reproduced in a temporary local probe, not committed.
For direct evidence from the final integration test itself, only the fixed
capacity expression was temporarily restored to the old hardcoded `true`.
The exact test `transparent_mandatory_routine_preserves_capacity_in_step_and_event`
then failed its expected-false step assertion. Fixed source was restored before
rerunning all five integration tests and the complete passing suite.

Verbatim output with the old expression:

```text
PROBE task  `Work Time` occupies_capacity=false
PROBE step  `Work Time` occupies_capacity=true
```

Verbatim output after the fix, from the same integration test:

```text
PROBE task  `Work Time` occupies_capacity=false
PROBE step  `Work Time` occupies_capacity=false
```

The test also asserts that Work Time's projected event is `transparent: true`.
The stored Task's flag remains false; the correction is in the step display
metadata, not in the kernel scheduling contract.

## Event list: integration test 1

Verbatim event array from the synthetic three-routine day:

```json
[{"color_id":"3","end_at":"2026-09-24T09:58:00Z","external_id":"01a0d6c30c177783b9534858d05ce741","reminders_minutes":[0],"start_at":"2026-09-24T09:28:00Z","summary":"Breakfast","task_id":"task_01a0d6c30c177783b9534858d05ce741","transparent":false},{"color_id":"9","end_at":"2026-09-24T12:28:00Z","external_id":"01a0d6c30c177783b9534868b1365839","reminders_minutes":[],"start_at":"2026-09-24T10:28:00Z","summary":"Work Time","task_id":"task_01a0d6c30c177783b9534868b1365839","transparent":true},{"color_id":"6","end_at":"2026-09-24T14:43:00Z","external_id":"01a0d6c30c177783b953487e047400bc","reminders_minutes":[10,0],"start_at":"2026-09-24T14:28:00Z","summary":"Standup","task_id":"task_01a0d6c30c177783b953487e047400bc","transparent":false}]
```

Assertions cover every event's category colour, transparency, reminder list,
Task-derived external id, and increasing start order. Breakfast is opaque with
colour 3 and reminder `[0]`; Work Time is transparent with colour 9 and no
reminders; Standup is opaque with colour 6 and reminders `[10,0]`. Planner-chosen
start times and generated Task ids are evidence from this run, not fixed outputs
required across separate generations.

## Existing expectations found by section A's search

Before editing, `rg -n 'occupies_capacity' tests` found 16 matching lines:

| Existing test file | Matching lines | Result |
|---|---:|---|
| `tests/static_category.rs` | 8 | Existing capacity/default and Static-step assertions unchanged |
| `tests/routine_planning.rs` | 4 | One assertion changed, described below |
| `tests/routine_instantiation.rs` | 2 | Template setup values unchanged |
| `tests/static_containment.rs` | 1 | Capacity expectation unchanged |
| `tests/quick_ubu_import.rs` | 1 | Imported template expectation unchanged |

**One existing test assertion moved:** `tests/routine_planning.rs:291`, inside
`f1_routine_day_preserves_committed_time_mandatory_order_and_idempotency`, changed
from expecting `occupies_capacity == true` to `false`. Its loop checks Check-in
1, 2 and 3 (synthetic source ids ending 101–103); all three fixture routines
already declare `transparent: true`. The old expectation encoded the shipped
bug. No other existing test or fixture changed. All existing tests pass.

No existing diagnostic code string was changed or removed. The new
`calendar_event_id_unmappable` diagnostic was searched across the entire
repository, including tests; no prior diagnostic was replaced.

## Additional verification

The six pure module tests cover valid and invalid external ids (including short,
long, wrong-prefix, uppercase, disallowed alphabet and non-ASCII cases), skipped
invalid ids, colour/transparency and event ordering, routine versus one-off
reminders, unchanged sets, sorted first-run creates, one changed event yielding
one update, and sorted deletes following creates and updates.

The five integration tests also verify:

- Missing, null, non-array, mixed-type, fractional, negative and i64-overflowing
  reminder lists yield empty lists without blocking preview. These deliberately
  malformed templates exist only in a synthetic in-memory database.
- A one-off Task captured through P1B-27's POST `/task` is projected with no colour
  or reminders and the default opaque event behavior.
- Preview matches `/calendar/current` for both an empty store and an admitted
  Plan with blocking deadline risk (`stale: true`), carrying its id unchanged.
- Two previews without replanning are byte-identical. Every operation is a
  create, ordered by external id, containing an event from the returned list.
  Snapshots of **every SQLite table** before and after prove no preview writes.
- A deliberately malformed Task id in a synthetic persisted Plan is skipped and
  named by `calendar_event_id_unmappable`, leaving the other events visible.

## Judgment calls and literal interpretations

No disagreement with the intended seven judgment calls. The following
ambiguities were resolved explicitly:

- Judgment call 6 says both “Only capacity-relevant Plan steps” and “every step”.
  The required transparent-routine case and section B's “one event per step”
  govern: every valid current Calendar step becomes an event, including
  `occupies_capacity: false`. A filter excluding those steps would contradict
  the transparency regression the ticket requires fixing.
- The test-count discrepancy is resolved in favor of all six required unit
  tests and all five required integration tests, yielding 222, not 217.
- The reminder helper returns the two requested BTreeMaps in a named
  `CalendarReminderMaps` result. Only `occurrence.routine_objective_id` links a
  Task to reminders; ordinary one-off Objective membership does not.
- Malformed reminder lists are all-or-nothing. Valid lists are nonnegative i64
  integers, matching the pinned Objective schema's minimum of zero. No sorting,
  deduplication, partial salvage, or Google-specific maximum was invented.
- Create/update operation bodies use `{kind, event}`; delete bodies use
  `{kind, external_id, summary}`. Missing event colour is serialized as null.
  The ticket left that wire layout unspecified; OpenAPI records it explicitly.
- The id function implements the supplied prefix, alphabet and length rule,
  rather than adding a stricter hex-only test. Event timestamps sort as their
  UTC `start_at` strings already supplied by the Calendar.
- The fix changes newly generated steps. Existing stored Plans are not rewritten;
  regenerate to obtain corrected flags in a Plan saved before this fix.
- Preview reads the current Plan plus current routine reminder templates. It
  does not replan, acquire an external observation, or persist an existing side
  for the diff. Event sets are keyed by external id; canonical Plan Task ids are
  assumed unique.
- No network means no remote publication of the otherwise complete local branch.

## Known limits

1. **Preview only.** Nothing is written to any calendar, and nothing records what was written before, so `operations` is always the full create set. The diff is real and tested; it has no persisted "existing" side until P1B-29.
2. **No deletion safety.** Google does not document whether an event id may be reused after the event is deleted. Because ids are derived from Task ids and a routine occurrence gets a fresh Task id each day, this is not reachable today, but a Task deleted and recreated with the same id would be.
3. **One calendar, no calendar selection.** Which Google calendar receives the projection is not modelled; that belongs with the client and its credentials in P1B-29.
4. **Colours come from the operator palette, not from Google.** `CategoryPalette` maps a category to a Google colour id by number. If the user's calendar uses a custom palette, the numbers still apply but the rendered colours may not be what the category name suggests.
5. **All-day events, recurrence and attendees are not modelled.** Every event is a timed, single, attendee-free block. UbU plans concrete spans, and a recurring Google event would fight the daily re-plan rather than help it.
