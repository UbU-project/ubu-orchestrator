# A seed for the mock Calendar

`UBU_CALENDAR_MOCK_EVENTS` names a JSON file that says what the mock
Calendar observes. It exists so that what the orchestrator does with an
observation can be asserted over HTTP, without a Google account. It is a test
affordance. It is an environment variable and not a route, so nothing about
it is on the production surface.

## What it changes, and what it does not

In `export_mode: "mock"` three paths build a recording Calendar client:
approve, capture and reconcile. Until P1B-47 that client was always built
from UbU's own applied record, so what the calendar "said" always equalled
what UbU believed it had applied.

| `UBU_CALENDAR_MOCK_EVENTS` | The mock client is built from |
|---|---|
| unset | the applied record, exactly as before |
| set | the events in the file |

**The seed replaces the observed set only.** The applied record, the desired
set and every decision rule are untouched. The fixture is what the calendar
says, not what UbU believes.

`GET /projection/calendar/preview` makes no Calendar call. It diffs the
desired set against the applied record, so the seed does not reach it.

A client injected by a test through `AppState::with_calendar_api` still takes
precedence over both.

The file is read once, at startup. To change what is observed, write a new
file and restart the orchestrator on the same store.

## The file

A JSON array of events in the shape `list_events` returns:

```json
[
  {
    "external_id": "5n0q8c9h7g4k2m1p3r6t8v0a2c",
    "summary": "Synthetic foreign meeting",
    "start_at": "2026-09-25T13:00:00Z",
    "end_at": "2026-09-25T13:30:00Z",
    "color_id": null,
    "transparent": false,
    "reminders_minutes": []
  }
]
```

`task_id` may be given. When it is omitted it is `task_<external_id>`, which
is what the wire parser gives an observed event. Capture and reconcile
restore the real Task id of an event UbU owns from the applied record, as
they do for a live observation.

Startup is refused, naming the path and the entry, when the file cannot be
read, is not JSON, is not an array, or holds an event that lacks a field,
does not end after it starts, or repeats an `external_id`:

```text
startup error: invalid mock Calendar events `<path>` (UBU_CALENDAR_MOCK_EVENTS), entry `1`: range start must precede end
```

A malformed category palette refuses startup in the same way.

## It cannot be used beside a live export

If the variable is set and a request asks for `export_mode: "live"`, the
request is refused with HTTP 409 before any Calendar client exists:

```json
{"code":"calendar_mock_seed_with_live_export","message":"This process was started with UBU_CALENDAR_MOCK_EVENTS set to `<path>`, a mock Calendar fixture, and the request asked for export_mode `live`; unset the variable and restart to use the live Calendar, or ask for `mock`"}
```

Neither the fixture nor the request is silently ignored. Nothing must be able
to confuse a fixture with the operator's real calendar. The refusal comes
before the checks for live configuration and live enablement.

## What it made reachable

Four behaviours could not be reached over HTTP in Mock mode before the seed,
because observed always equalled applied:

| Behaviour | Built in | Seeded with |
|---|---|---|
| A colour on a Dynamic event completes its Task | P1B-33 | the applied events, with a colour on the Dynamic one |
| A moved Static event moves the Task's window | P1B-34 | the applied events, with the Static one at another window |
| An event UbU never applied is `foreign` | P1B-31, P1B-32 | the applied events and one more |
| A recurring instance is captured as occupied time UbU does not own | P1B-44, P1B-51 | an event whose id has the `{base32hex}_{timestamp}` shape |

`ubu-devshell/scripts/check-ui-contract.sh` asserts all four, as scenarios 7
to 10.

## Limits

1. The seed is an environment variable, so changing it needs a restart.
2. An empty value is a path and refuses startup. Unset the variable to turn
   the seed off.
3. The mock client is rebuilt from the file on every request. What one
   request inserts into it is not there for the next.
