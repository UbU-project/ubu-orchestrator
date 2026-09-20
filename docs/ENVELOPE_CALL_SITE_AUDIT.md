# P1B-6 registration and canonical mutation audit

Every production canonical mutation uses `AppState::envelope_for`, which delegates
assembly/recording times and key issuance to the pinned `LocalIssuer`. The actor
is always `DeviceRegistration.registered_identity_id`; the issuer is constructed
from that same registration's Device id. Existing `AuthoritySource` values remain.

## Production call sites

`T` means the target's canonical UbuId. All paths below are relative to this repo.
There are 13 production call expressions after replacing the direct SQL Task
completion update, rather than the ticket's approximate 18. There are no direct
canonical writer calls in `src/api`; those endpoints delegate to these services.

| File:line | Writer | Create/update | observed_versions | AuthoritySource | effective_time |
|---|---|---|---|---|---|
| src/services/bootstrap_service.rs:190 | admit_object | Objective create | T: Absent | User | Sampled operator/bootstrap creation time |
| src/services/bootstrap_service.rs:293 | admit_object | UniverseState create | T: Absent | User | Initial captured_at |
| src/services/bootstrap_service.rs:327 | admit_object | Preference create | T: Absent | User | Sampled operator/bootstrap creation time |
| src/services/import_service.rs:100 | admit_object | Task create | T: Absent | Existing import authority (System) | Mapped source provenance.created_at (issue/PR/CI updated_at); fixture-only import uses wall clock |
| src/services/import_service.rs:274 | store_external_reference | Append | Empty | Existing reference provenance (System) | Source observed_at |
| src/services/log_service.rs:79 | append_log_entry | Recorded action append | Task: observed Version(n), after completion when applicable | User for complete/snooze; UserOverride for override | One operator action time, shared with completion/effects |
| src/services/log_service.rs:144 | append_log_entry | Legacy action append | Empty | User | Operator action wall clock |
| src/services/log_service.rs:247 | admit_object | Task completion update | Task: Version(n) from load_task | Existing action authority (User) | Operator completion time |
| src/services/log_service.rs:342 | persist_universe_state | Effects update | UniverseState: Version(n); completed Task: Version(n) | Existing completing action authority (User) | Same completion time |
| src/services/planning_service.rs:605 | append_log_entry | Blocking-risk trigger append | Empty | System | Generated trigger's wall-clock time |
| src/services/projection_service.rs:459 | admit_object | ExternalEvent create | T: Absent | Request's existing authority | Operator acceptance/occurred_at time |
| src/services/projection_service.rs:849 | append_log_entry | Boundary-decision append | Empty | Existing boundary decision authority (AutomationWorker) | Boundary decision effective_time |
| src/services/recalculation_service.rs:148 | append_log_entry | Recalculation trigger append | Empty | System | Request triggered_at |

## Test mutation call sites

Test fixture writers also use the state issuer; no test hand-builds an envelope.
These locations supplement the production table, including intentional failures.

| File:line | Writer | Create/update | observed_versions | AuthoritySource | effective_time |
|---|---|---|---|---|---|
| tests/next_action.rs:242 | admit_object | Mark fixture Task failed | T: observed Version(n) | User | Wall clock |
| tests/next_action.rs:369 | admit_object | Objective create | T: Absent | User | Fixture creation time |
| tests/next_action.rs:421 | admit_object | Task create | T: Absent | User | Fixture creation time |
| tests/planning_o9.rs:867 | admit_object | Task create | T: Absent | User | Fixture creation time |
| tests/planning_o9.rs:893 | admit_object | UniverseState create | T: Absent | User | Fixture creation time |
| tests/planning_o9.rs:928 | admit_object | Preference create | T: Absent | User | Fixture creation time |
| tests/planning_o9.rs:966 | admit_object | Snapshot create | T: Absent | User | Supplied observed_at |
| tests/planning_o9.rs:1042 | append_log_entry | Override fixture append | Empty | UserOverride | Fixture creation time |
| tests/user_actions.rs:270 | admit_object | UniverseState create | T: Absent | User | Fixture creation time |
| tests/user_actions.rs:296 | admit_object | Task-with-effects create | T: Absent | User | Fixture creation time |
| tests/user_actions.rs:331 | admit_object | Task create | T: Absent | User | Fixture creation time |
| tests/mutation_envelopes.rs:120 | admit_object | Fixture effects update | T: observed Version(n) | User | Wall clock |
| tests/mutation_envelopes.rs:237 | admit_object | Create | T: Absent | User | Test's sampled time |
| tests/mutation_envelopes.rs:249 | admit_object | Duplicate create, rejected | T: Absent | User | Same test time |
| tests/mutation_envelopes.rs:263 | admit_object | Update | T: Version(1) | User | Same test time |
| tests/mutation_envelopes.rs:276 | admit_object | Stale update, rejected | T: Version(1), actual 2 | User | Same test time |
| tests/mutation_envelopes.rs:280 | admit_object | Update replay | Original T: Version(1), replay precedes checks | User | Original envelope's time |

## Literal readings and bounded compatibility changes

- First-run registration mints both a Device id and its operator Identity once,
  storing them together. No separate actor configuration or admitted Identity
  object is invented. Defaults are `OsUserProfile`, label `Local UbU operator
  enclave`, `Registered`, `LocalOnly`, zone `local-workspace`, capability
  `admission`, empty Compartment allowlist, and no last-seen time. These describe
  the local operator enclave without inferring hardware identity or access grants.
- UBU_DEVICE_REGISTRATION is a filesystem path; absent configuration defaults to
  `ubu-device-registration.json` beside UBU_DB_PATH. Plain paths and ordinary
  sqlite:/sqlite:// path spellings with query suffixes are supported. For an
  in-memory database the default is the working directory; the explicit in-memory
  AppState constructors do not consult or create this file at all.
- Existing material is inspected with symlink_metadata, so dangling symlinks and
  unreadable/invalid files cannot silently trigger re-registration. Exclusive
  create_new avoids overwriting concurrently registered identities; a competing
  file is restored or rejected if incomplete. Writes flush via sync_all; Unix
  mode is 0600. Parent directories must already exist. Write failures refuse
  startup and do not erase potentially recoverable material. Only a successful
  first write logs the NEW Device warning, never the full registration document.
- Revocation is checked during file loading and both state-construction paths,
  before database access. Valid restored files are not rewritten. Runtime file
  watching/revocation refresh is not added. In-memory tests can inject registration;
  the backward-compatible convenience constructor generates ephemeral material.
- LocalIssuer's published semantics use one clock sample for created_time and
  recorded_time at issuance immediately before the store call, not a new clock
  sampled inside SQLite commit. Domain effective_time is supplied independently.
  observed_policy_versions and execution_context remain None; no policy or Zone
  enforcement is introduced.
- The audit found one production SQL UPDATE of objects in Task completion. It is
  replaced with ordinary envelope-aware admission, preserving the read shell and
  asserting its version. One analogous fixture SQL update is likewise replaced.
  The shared UniverseState reader now returns payload and version from one query;
  planning discards the version, while effects use it. Each write remains a store
  transaction; the existing multi-step completion workflow is not made one new
  cross-operation transaction or given automatic retries.
- Recorded-action logs depend on the Task version whose status they describe.
  Effects depend on the just-completed Task and the UniverseState being changed.
  Other log appends record caller intent, a trigger, or a derived preview decision
  without asserting canonical object state. Reference ids alone are not version
  dependencies. Plans/previews are not versioned canonical targets.
- Imported Task effective_time comes from the pinned mapper's provenance timestamp,
  which is the GitHub issue/PR/CI updated_at. Source-less fixtures have no domain
  timestamp and use wall clock. External-reference observed_at is retained.
  Completion has no client-supplied timestamp, so one sampled operator action time
  is reused for its Task mutation, effects, and decision log. Accepted projection
  changes have no separate source timestamp, so operator acceptance time is used.
- AuthoritySource follows existing provenance: bootstrap and ordinary actions User;
  override UserOverride; import, recalculation, and risk triggers System; projection
  acceptance uses the request; boundary decisions AutomationWorker. The projection
  gate's former random actor Identity is replaced with the registration Identity.
- The pinned GitHub adapter still uses an older ubu_core revision. Its Task and
  ExternalReference values cross through their existing JSON contracts into the
  current core types; export authority crosses back via its unchanged wire enum.
  No adapter/planning revision or dependency declaration is added or changed.
  Older core revisions necessarily remain in Cargo.lock as transitive dependencies.
- Existing assertions are retained. Fixture signature updates and the mode-test
  setup merely supply AppState/issuance context. Tests use in-memory GitHub fakes;
  default modes are explicitly set to mock for verification. No candidate writers,
  candidate review APIs, sibling changes, or live GitHub test calls are introduced.

## Audit commands

```sh
rg -n 'queries::(admit_object|persist_universe_state|append_log_entry|store_external_reference)' src tests
rg -n 'INSERT INTO|UPDATE |DELETE FROM|REPLACE INTO' src tests
rg -n 'MutationEnvelope \{' src tests
rg -n 'store_advisory_candidate|transition_advisory_candidate|reject_advisory_candidate|admit_advisory_candidate' src tests
```

The latter two searches have no matches. Remaining production SQL mutations are
of derived plans and projection bookkeeping tables, unchanged by this ticket.
