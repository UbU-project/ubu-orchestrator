# Live Google Calendar projection

P1B-30 connects the existing gated Calendar apply to Google. Preview and the
applied-set diff remain local. Mock mode still uses the recording client; Live
always selects GoogleCalendarApi. The CalendarApi trait is unchanged.

## Operator configuration and session enablement

| Variable | Meaning |
|---|---|
| `UBU_GOOGLE_CREDENTIALS_PATH` | Absolute path to the existing installed-app Google OAuth application JSON used by Quick UbU. |
| `UBU_GOOGLE_TOKEN_CACHE_PATH` | Absolute path to the yup-oauth2 token cache the operator permits UbU to read and update. A missing file can be created after consent; its parent directory must exist and be writable. |
| `UBU_GOOGLE_CALENDAR_ID` | Destination calendar ID, default `primary`. Use a scratch calendar for the first live run. |

Paths are optional at server startup, so offline operation needs no Google
configuration. Missing/empty paths refuse Live with HTTP 503
`calendar_live_export_unconfigured`. Unreadable/invalid credentials or cache
produce an operation failure naming the corresponding environment variable,
without echoing paths, credential contents, OAuth errors or provider error bodies.
Reads happen only when a permitted live operation needs a token. Neither secrets
nor tokens enter the StateStore, projection payloads, boundary Log or responses.
Credential/cache files remain operator-owned files outside the database and repo.

Configuration does not enable export. Every fresh process starts disabled. Enable
this process with the following request (no secret is accepted):

```sh
curl --fail-with-body -sS http://127.0.0.1:7878/desktop/session/google-calendar \
  -H 'Content-Type: application/json' \
  -d '{"schema_version":"ubu.orchestrator.desktop_session.v1"}'
```

The response has `accepted: true` and `enabled: true`. Repeating the request is
harmless; restarting the process loses enablement. Without it, configured Live
apply returns HTTP 403 `calendar_live_export_not_enabled`. Enablement itself does
not read credentials, perform OAuth, change the database, or export events.

The reason is the existing `src/api/desktop.rs` TODO:

> TODO(phase2-tauri-bridge): This mutating loopback endpoint is intentionally
> left without per-run bearer-token or CSRF defenses while the temporary HTTP
> bridge remains in Phase 1.

This flag is an explicit session opt-in, not authentication or a CSRF defense:
anything able to reach the endpoint could enable it. The existing per-operation
Legitimizer gate remains required, including worker authority and permitting
policy. Enablement does not bypass that gate.

OAuth uses yup-oauth2 9's installed HTTP redirect flow and `CALENDAR_SCOPE`
`https://www.googleapis.com/auth/calendar`, matching Quick UbU. A usable cache
avoids new consent. On first use, the desktop browser opens for consent; Linux
requires DISPLAY or WAYLAND_DISPLAY and xdg-open (macOS uses open; Windows uses
rundll32). A headless run needing consent fails without a terminal prompt. Browser
launch failure aborts consent explicitly because this version of yup-oauth2
ignores its delegate's returned error in HTTPRedirect mode. Consent/refresh is
bounded to five minutes; Calendar requests to 30 seconds. There are no HTTP
redirects or application retries except the specified insert-to-patch transition.

The default OAuth delegate prints its consent URL, which includes the client ID;
UbU replaces it. A mandatory logging filter excludes OAuth and HTTP/TLS library
targets even with verbose RUST_LOG settings, because yup-oauth2's debug logs can
contain token values. Transport errors are sanitized before persistence. Browser
launch output is discarded.

## Event body

| DesiredEvent | Google writable field |
|---|---|
| `summary` | `summary` string |
| `start_at` | `start: {dateTime: <RFC 3339>}` |
| `end_at` | `end: {dateTime: <RFC 3339>}` |
| `color_id` | `colorId` string; omitted entirely when absent |
| `transparent = true` | `transparency: "transparent"` |
| `transparent = false` | `transparency: "opaque"` |
| `reminders_minutes` | `reminders: {useDefault: false, overrides: [{method: "popup", minutes: <integer>}, ...]}` |

GoogleEventBody matches Quick UbU's writable body field for field, including
field order, disabled defaults, empty overrides, and colour omission. These are
the fields already proven to display correctly on the operator's phone. Reminder
minutes keep DesiredEvent's i64 representation without narrowing Quick UbU's i32
wire integers. RFC 3339 strings retain their supplied offset spelling.

Insert adds the Google `id` property from `external_id` so P1B-28's task-derived
IDs remain stable; patches send only the body above. Unlike Quick UbU's create
path, UbU must supply an ID rather than accept a generated one. Both calendar and
event path segments are percent-encoded, including `@` and `/`.

The pure reader tolerates additional Google fields, preserves the writable fields
and reconstructs `task_` plus the external ID. It requires a valid task-mappable
ID, a summary string, timed RFC 3339 start/end with positive duration, and explicit
popup reminders (`useDefault: false`). Missing empty overrides are accepted;
absent transparency defaults to opaque as in Quick UbU. Cancelled, all-day,
unmappable, or malformed entries are skipped individually with a diagnostic.
Calendar-default reminders cannot be reconstructed without the calendar defaults,
so those entries are skipped rather than guessed. Pagination follows nextPageToken
and detects repeats; no caller uses list_events yet.

## Status handling

| Operation | HTTP status | Result |
|---|---|---|
| Any | 2xx (including 200 and 204) | Success |
| Delete | 404 or 410 | Success: already gone |
| Insert | 409 | Record `calendar_insert_converted_to_patch`, then patch the same ID once |
| Patch after conflict | 2xx | Success, original create operation recorded applied |
| Any remaining combination | Non-2xx | Per-operation failure, retained for the next preview |

A patch 409 does not loop; insert/patch/list 404 or 410 fails. Google failure bodies
are available only to the pure outcome classifier and are never copied into
persisted diagnostics. The ordinary `calendar_operation_failed` result names the
operation, event ID and HTTP status. Conflict conversion remains visible even if
the following patch fails. P1B-29's partial-apply snapshot includes only successes.

## Manual scratch-calendar smoke procedure

**This is the only procedure that exercises the transport. CI and the ticket's
automated tests never open a network socket. Never run the first live apply
against the primary calendar or a calendar you rely on.** This procedure is for
the operator, and was not run by Codex.

Prerequisites: a desktop browser, curl, jq, Python 3 and Cargo. In Google Calendar's
web settings, create an empty calendar named "UbU scratch". Copy its Calendar ID
from Settings → Integrate calendar. Confirm the credential path uses the existing
Quick UbU OAuth app, and choose a writable token cache path you are willing to
have updated. Do not put any of these files inside a repository.

1. In the server terminal, set these values yourself. Replace the placeholders;
   do not paste credentials or cache contents into commands, chats or tickets.
   Use a fresh, separate scratch database and registration directory. Mock and
   Live currently share an applied-set history, and that history is not keyed by
   calendar ID: do not reuse a mock/production database or change its destination.

   ```sh
   mkdir -p /tmp/ubu-calendar-smoke
   export UBU_DB_PATH=/tmp/ubu-calendar-smoke/state.db
   export UBU_GOOGLE_CREDENTIALS_PATH='/absolute/operator/path/credentials.json'
   export UBU_GOOGLE_TOKEN_CACHE_PATH='/absolute/operator/path/token-cache.json'
   export UBU_GOOGLE_CALENDAR_ID='<paste the scratch calendar ID here>'
   cargo run --locked
   ```

   If the scratch DB already exists, choose a new directory. Confirm that
   UBU_DEVICE_REGISTRATION is unset or also points at a dedicated scratch file.
   Leave this server running. The phone must have the scratch calendar selected
   for display and synced in the same Google account used during OAuth consent.

2. In a second terminal, from the orchestrator repository, prepare synthetic data
   for tomorrow in UTC. These commands create only synthetic fixture/request
   files, never credentials. One routine exercises popup reminders and transparent
   availability, and two ordinary static Tasks exercise update and deletion.

   ```sh
   export UBU_SMOKE_DAY="$(date -u -d tomorrow +%F)"
   python3 - <<'PY'
   import json, os, pathlib
   out = pathlib.Path('/tmp/ubu-calendar-smoke')
   day = os.environ['UBU_SMOKE_DAY']
   routine_id = '00000000-0000-4000-8000-000000000301'
   routine = dict(id=routine_id, title='Synthetic reminder', recurrence='Daily',
                  start_time='09:00:00', duration=[900, 0], dynamic=False,
                  category='work', transparent=True, reminders=[10, 0])
   snapshot = dict(snapshot_version=1, task_origins={}, store=dict(
       routines={routine_id: routine}, tasks={}, objectives={}, bundles={}, preferences=[]))
   (out/'snapshot.json').write_text(json.dumps(snapshot))
   for name, hour in [('update', '10'), ('delete', '11')]:
       task = dict(schema_version='ubu.orchestrator.task_capture.v1',
                   title='Synthetic '+name, category_tag='personal', tags=['personal'],
                   occupies_capacity=True,
                   static_window=dict(start=f'{day}T{hour}:00:00Z', end=f'{day}T{hour}:30:00Z'))
       (out/f'{name}.json').write_text(json.dumps(task))
   (out/'plan.json').write_text(json.dumps(dict(horizon=dict(
       start=f'{day}T00:00:00Z', end=f'{day}T23:59:59Z'))))
   PY
   curl --fail-with-body -sS http://127.0.0.1:7878/import/quick-ubu \
     -H 'Content-Type: application/json' \
     -d '{"snapshot_path":"/tmp/ubu-calendar-smoke/snapshot.json","timezone":"UTC"}'
   for name in update delete; do
     curl --fail-with-body -sS http://127.0.0.1:7878/task \
       -H 'Content-Type: application/json' --data-binary @/tmp/ubu-calendar-smoke/$name.json \
       > /tmp/ubu-calendar-smoke/$name-result.json
   done
   curl --fail-with-body -sS http://127.0.0.1:7878/planning/generate \
     -H 'Content-Type: application/json' --data-binary @/tmp/ubu-calendar-smoke/plan.json
   ```

3. Enable this process using the session request above. Then preview:

   ```sh
   curl --fail-with-body -sS http://127.0.0.1:7878/projection/calendar/preview \
     > /tmp/ubu-calendar-smoke/preview.json
   jq '{stale,events,operations,diagnostics}' /tmp/ubu-calendar-smoke/preview.json
   ```

   Confirm three creates, the synthetic titles, expected times, colour IDs,
   transparent reminder, opaque ordinary Tasks, and `[10,0]` reminder minutes.
   Stop if the preview is stale, contains unexpected tasks, or has diagnostics.
   Once reviewed, apply exactly that preview:

   ```sh
   jq '{schema_version:"ubu.orchestrator.calendar_projection_approval.v1",
        preview_id,authority_source:"automation_worker",export_mode:"live"}' \
     /tmp/ubu-calendar-smoke/preview.json > /tmp/ubu-calendar-smoke/approve.json
   curl --fail-with-body -sS http://127.0.0.1:7878/projection/calendar/approve \
     -H 'Content-Type: application/json' --data-binary @/tmp/ubu-calendar-smoke/approve.json
   ```

   Complete browser consent if prompted. HTTP 200 alone is insufficient: verify
   `status: "applied"`, three applied operations and no unexpected diagnostics.
   For a partial result, inspect failures and get a fresh preview before applying
   again. Confirm all three events on the phone in the scratch calendar. Inspect
   colour, the reminder's two popup notifications (10 minutes before and at start),
   and free/transparent versus busy/opaque availability. Match displayed local
   times to the UTC timestamps. A fresh preview should now contain zero operations.

4. Change the first ordinary Task's title and complete the second, then regenerate
   the same day. Completing removes the Task from the projected set and therefore
   tests deletion of its Calendar event; it does not erase canonical history.

   ```sh
   UBU_SMOKE_UPDATE_ID=$(jq -r .task_id /tmp/ubu-calendar-smoke/update-result.json)
   UBU_SMOKE_DELETE_ID=$(jq -r .task_id /tmp/ubu-calendar-smoke/delete-result.json)
   jq '{schema_version:"ubu.orchestrator.task_capture.v1",expected_version:.version,
        title:"Synthetic updated"}' /tmp/ubu-calendar-smoke/update-result.json \
     > /tmp/ubu-calendar-smoke/edit.json
   curl --fail-with-body -sS -X PATCH "http://127.0.0.1:7878/task/$UBU_SMOKE_UPDATE_ID" \
     -H 'Content-Type: application/json' --data-binary @/tmp/ubu-calendar-smoke/edit.json
   curl --fail-with-body -sS "http://127.0.0.1:7878/task/$UBU_SMOKE_DELETE_ID/action" \
     -H 'Content-Type: application/json' \
     -d '{"schema_version":"ubu.orchestrator.task_action.v1","action":"complete"}'
   curl --fail-with-body -sS http://127.0.0.1:7878/planning/generate \
     -H 'Content-Type: application/json' --data-binary @/tmp/ubu-calendar-smoke/plan.json
   ```

   Repeat step 3's preview commands. Confirm exactly one update and one delete,
   then approve the new preview using step 3's apply commands. Confirm the renamed
   event and the removed event on the phone, with the reminder unchanged. Do not
   use the legacy `/reject` endpoint for this step; it only records a decision.

5. Stop the scratch orchestrator. In Google Calendar settings, permanently delete
   the scratch calendar after checking the selected calendar's name. Remove only
   the dedicated scratch DB/request files if desired; retain the operator-owned
   OAuth app and cache. Record the outcome and sanitized operation diagnostics,
   never credentials or tokens. For subsequent real use choose the intended
   calendar explicitly and use its own database/applied-set history.

## Known limits

1. **The transport itself is unverified by CI.** Everything above `send()` is tested; the socket is not. Only §F's manual procedure covers it.
2. **No reconciliation.** UbU still projects its own belief. An event edited or deleted in Google is not noticed, and `list_events` is implemented but nothing calls it yet. That is P1B-31.
3. **The OAuth flow needs a browser once.** `InstalledFlowAuthenticator` opens a consent flow the first time and caches the token. On a headless run with no cached token, apply fails rather than blocking on a prompt.
4. **No rate limiting or backoff.** Google's per-user quota is generous relative to one person's day, but a large first apply is a burst. A failed operation is recorded and re-proposed, not retried.
5. **One calendar, all-or-nothing.** Every projected event goes to the single configured calendar. There is no per-category or per-compartment routing.
6. **Deleting a Task does not delete its event until the next apply.** The applied set changes only when apply runs, so the phone can be stale between applies.

