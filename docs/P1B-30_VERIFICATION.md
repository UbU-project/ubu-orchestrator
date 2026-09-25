# P1B-30 verification

## Scope and checks

Only `ubu-orchestrator` changed, from clean HEAD
`e85703ea3fc8b35a4b6f4a5dc9fa05f9f6428fcb`, on branch
`p1b-30-calendar-live`. All 11 sibling repositories retained their original HEADs
and clean statuses. All required pinned siblings match the ticket; no pin changed.
Quick UbU's `gcal/src/lib.rs` was read in full, along with the specified orchestrator
and adapter reference files. No actual Google credential or cache file was opened.

**239 tests passed; zero failed or ignored**, up from 231. The new
`tests/calendar_wire.rs` contains exactly eight tests. Count = sum of the `passed`
counts from Cargo's unit, integration and doc-test result lines; zero-test targets
add zero. The existing seven Calendar apply tests also passed. Sections A through
E each passed the full 231-test suite before commit. F changed documentation only
and retained E's green code state; G passed the full 239-test suite. No compilation
concurrency, memory or other runtime caps were introduced.

Commands used a sanitized environment (`env -i` with only PATH, HOME and
CARGO_NET_OFFLINE=true), excluding real Google/GitHub settings. All Cargo commands
ran offline, with `--locked` after A resolved the new dependencies from the local
cache. No dependency-fetch network was needed.

Clippy before and after used the identical command in this checkout and run:

```sh
cargo clippy --locked --offline --all-targets --message-format=json
```

Counting method: take JSON `compiler-message` records for package
`ubu_orchestrator` with level `warning`, deduplicate by `(code, message, primary
span file/line/column)`. Deduplication removes repeated lib/lib-test emissions;
dependency warnings and Cargo summary lines do not count. **10 before, 10 after,
delta 0**. The warning identities are unchanged. No new lint allowances were added.
The temporary unused-import warning during G's first compilation was removed
before final clippy and the full suite.

Cargo.toml gained exactly the two dependency declarations required by section A.
Cargo.lock gained **36 package entries**, comparing `(name, version, source)` sets.
Every existing entry remains at its original version and source, with its original
checksum. No existing package changed version, and no Git dependency revision
moved. Existing lock dependency references gain version qualifiers where the new
packages introduce duplicate names. Neither native-tls nor OpenSSL was added
(`openssl-probe` is a certificate-location helper, not OpenSSL).

`cargo run --locked --offline --example generate_openapi` regenerated
`openapi/openapi.generated.json`, including the session endpoint, enablement
schemas and live refusal statuses. `git diff --check` passed. Rustfmt was run only
on the three newly created Rust files; individual-path `rustfmt --check --edition
2021` passed. No whole-repository formatting was performed.

## Offline and secret handling evidence

No test performed a network call. Every suite ran under an inherited seccomp
filter rejecting IPv4 and IPv6 `socket()` creation. The final complete run also
used `strace -f -qq -e trace=network` around Cargo and every descendant. The trace
contains **zero AF_INET/AF_INET6 socket attempts and zero connect() calls**. Its only
socket family is AF_UNIX, for local build-tool IPC. Tests use pure fixture values,
SQLite in memory and Axum Router::oneshot; the live refusal tests stop before
constructing GoogleCalendarApi. No OAuth flow or live HTTP client method ran.
The final command was:

```sh
env -i PATH="$PATH" HOME="$HOME" CARGO_NET_OFFLINE=true   python3 /tmp/p1b-30-results/offline-test.py   strace -f -qq -e trace=network -o /tmp/p1b-30-results/G-network.log   cargo test --locked --offline -- --nocapture
```

The temporary wrapper loads libseccomp, installs EPERM rules for socket domain
AF_INET and AF_INET6, then execs the supplied command. It changes no repository or
runtime configuration. Tracing required sandbox escalation because ptrace is not
available in the ordinary sandbox. A–G evidence, baseline snapshots and clippy
JSON logs are under `/tmp/p1b-30-results` on the working machine.

No real credential, access/refresh token, client ID, client secret or calendar ID
appears anywhere in the diff. Verification combined reviewing all changed/new
files, tracing each credential/token value's path, and scanning for Google token,
client-secret, OAuth client-ID and calendar-ID patterns. No pattern hits were
found. Fixture account/calendar addresses use example.invalid, the only header
value in tests is explicitly synthetic, and documentation uses placeholders.
Credentials/cache are read only after an allowed live operation and remain in
memory or the operator-owned yup-oauth2 cache, never the StateStore or a response.
No provider error body or library error Display string is persisted or logged.

The OAuth library's default delegate would print the consent URL, and its debug
logs can include cached tokens. The custom browser delegate suppresses that
output, while the application logging filter unconditionally drops OAuth and
HTTP/TLS library targets even when RUST_LOG asks for them. This additional change
to `src/tracing.rs` is necessary to satisfy the ticket's secrecy requirement.
Browser output is discarded. These OAuth/browser and transport I/O paths are
reviewed code and manual-smoke scope, not exercised by these tests.

## Required verbatim evidence

Expected body fixture (the checked-in file has exactly these bytes, without a
trailing newline):

```json
{"summary":"Synthetic focus","start":{"dateTime":"2026-09-25T09:00:00Z"},"end":{"dateTime":"2026-09-25T09:30:00Z"},"colorId":"5","transparency":"transparent","reminders":{"useDefault":false,"overrides":[{"method":"popup","minutes":10},{"method":"popup","minutes":0}]}}
```

Test 7 output, verbatim after its `P1B30_OUTCOME ` marker:

```text
404 | synthetic provider failure | AlreadyGone
410 | synthetic provider failure | AlreadyGone
409 | synthetic provider failure | Conflict
200 | synthetic provider failure | Ok
204 | synthetic provider failure | Ok
400 | synthetic provider failure | Failed("synthetic provider failure")
500 | synthetic provider failure | Failed("synthetic provider failure")
```

In the same test, all four operations are checked across HTTP statuses 100–599.
Only delete accepts AlreadyGone; only insert converts Conflict to patch, so a
conflicting patch cannot loop. Failures include operation/ID/status and exclude
the provider body. Request methods, bearer/JSON headers, insert-only ID, empty
reminder overrides, malformed field rejection, pagination and repeated-token
rejection are also tested within the eight required tests.

Test 8 output, verbatim after its `P1B30_REFUSAL ` marker:

```text
503 Service Unavailable {"code":"calendar_live_export_unconfigured","message":"Live Calendar export requires UBU_GOOGLE_CREDENTIALS_PATH and UBU_GOOGLE_TOKEN_CACHE_PATH"}
403 Forbidden {"code":"calendar_live_export_not_enabled","message":"Live Calendar export is not enabled for this process; POST /desktop/session/google-calendar first"}
```

Both refusals have zero recorder calls, zero delivery records and zero boundary
Log entries. Test 8 additionally verifies default primary, path/calendar builders,
secret-free enablement with nonexistent fixture paths, shared enablement across
AppState clones, no enablement Log persistence, missing-schema rejection, and
disabled state in a newly constructed process state.

Before replacing `calendar_live_export_unavailable`, a whole-repository search
including hidden files and tests (excluding .git/target) found four occurrences:
client source, `tests/calendar_apply.rs`, CALENDAR_APPLY.md, and the historical
P1B-29 verification report. The existing apply test's two expectations changed
from HTTP 501/unavailable to HTTP 503/unconfigured; all no-call/no-record assertions
remain. CALENDAR_APPLY.md now describes live enablement and links the new guide.
The historical P1B-29 result was preserved as historical evidence.

## Judgment calls and literal interpretations

No implementation disagreement with calls 1–9; the qualifications below are
explicit rather than silent deviations.

| Call | Assessment |
|---|---|
| 1. Pure wire model | Agreed. URL/body/header/method construction, parsing, pagination, status interpretation and conflict transitions are pure. The transport executes that policy; OAuth, browser, file and socket I/O remain manual-smoke scope. |
| 2. reqwest/rustls | Agreed; exact requested declaration. Measured incremental lock cost is 36 packages, including yup-oauth2's older hyper/rustls versions. |
| 3. Installed yup-oauth2 | Agreed; HTTPRedirect, application-secret reader, persisted cache and exact Calendar scope match Quick UbU. |
| 4. Operator configuration | Agreed. Absent paths do not fail offline startup. Missing paths refuse Live; invalid/unreadable material fails a permitted operation with the variable name, preserving the existing per-operation result path. |
| 5. Per-session enablement | Agreed as an opt-in, with a security qualification: an unauthenticated enable endpoint cannot authenticate the operator or prevent CSRF/local clients from enabling it. It does not replace the existing export gate. Full HTTP bridge defenses remain deferred, as requested. |
| 6. Delete 404/410 | Agreed; only delete treats them as success. |
| 7. Insert 409 to patch | Agreed; exactly one transition, with calendar_insert_converted_to_patch drained into the persisted result even when patch fails. CalendarApi remains unchanged. |
| 8. One calendar | Agreed; default primary and explicit operator override. Manual first use requires a separate scratch calendar. |
| 9. Quick UbU body | Agreed; exact serialized writable fields and order. Insert additionally supplies the already-specified task-derived ID; event_body itself remains identical. |

Ambiguities were resolved literally:

- Exact body versus deterministic IDs: add `id` only in the pure insert request,
  not GoogleEventBody. Patches use precisely the common writable body.
- Reminder integer type: preserve DesiredEvent's i64 without truncation; serialized
  JSON integers match Quick UbU's i32 field for supported values. Google validation
  failures follow the normal operation failure path.
- State-aware refusal statuses were unspecified: use 503 for missing configuration
  and 403 for disabled Live, both before loading previews or invoking clients.
- The enable endpoint mirrors the GitHub session schema and accepts only
  schema_version; no credentials or arbitrary fields. It checks configured paths
  but does not read files or contact OAuth.
- Unreadable-path handling is lazy because absent paths must not break offline
  startup and the gate must precede external effects. Error messages have the
  requested variable-name shape; failed permitted operations use P1B-29 results.
- RFC 3339 input strings retain their offset spelling. Google may add fields;
  the inverse needs a task-mappable ID, summary, timed positive-duration interval
  and explicit popup reminders. Missing empty overrides and absent transparency
  are accepted. Defaults cannot be reconstructed without calendar defaults;
  all-day/cancelled/unmappable/default-reminder entries are skipped individually.
- The pure Failed outcome carries the body exactly as requested. The operation
  policy deliberately emits only HTTP status, operation and ID, since persisting
  arbitrary response bodies could disclose secrets.
- No browser on headless first use: a failure notification aborts HTTPRedirect,
  whose library implementation otherwise ignores delegate errors. Consent waits
  are bounded; no stdin prompt or consent URL is printed.
- The existing applied set is not keyed by mock/live mode or calendar ID. The
  manual procedure therefore uses a fresh scratch database and warns against
  changing an existing database's destination. Reconciliation/routing is not added.

## Known limits (verbatim from the ticket)

1. **The transport itself is unverified by CI.** Everything above `send()` is tested; the socket is not. Only §F's manual procedure covers it.
2. **No reconciliation.** UbU still projects its own belief. An event edited or deleted in Google is not noticed, and `list_events` is implemented but nothing calls it yet. That is P1B-31.
3. **The OAuth flow needs a browser once.** `InstalledFlowAuthenticator` opens a consent flow the first time and caches the token. On a headless run with no cached token, apply fails rather than blocking on a prompt.
4. **No rate limiting or backoff.** Google's per-user quota is generous relative to one person's day, but a large first apply is a burst. A failed operation is recorded and re-proposed, not retried.
5. **One calendar, all-or-nothing.** Every projected event goes to the single configured calendar. There is no per-category or per-compartment routing.
6. **Deleting a Task does not delete its event until the next apply.** The applied set changes only when apply runs, so the phone can be stale between applies.


The manual smoke procedure is documented in [CALENDAR_LIVE.md](CALENDAR_LIVE.md)
and **was not executed**. The operator must confirm his existing OAuth app and
writable cache path and create the scratch calendar before any live run. No live
success or phone verification is claimed.

## Section commits

One signed commit per lettered section; no force-push:

```text
3665666 P1B-30 A: add Calendar HTTP and installed OAuth dependencies
1ba48ed P1B-30 B: configure Google credential paths and calendar
f661c5d P1B-30 C: isolate Calendar wire model and response policy
3932839 P1B-30 D: add Google Calendar transport and private OAuth handling
f9ad388 P1B-30 E: gate live Calendar export on process enablement
e9cd5d6 P1B-30 F: document live Calendar setup and scratch smoke procedure
```

The seventh commit, section G, contains the eight tests, two synthetic JSON
fixtures, regenerated OpenAPI and this verification report.
