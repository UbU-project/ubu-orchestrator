# P1B-10b verification

## E: tests and compatibility

All three target trees were clean before edits. The setup command removed the
generated `.cargo/config.toml` files, as explicitly required by this ticket.
No Cargo patch config was present for any build or test. Dependencies resolve
from committed git revisions; no path overrides or new packages were added.

Full suites passed:

- ubu-core: 112 tests, including specific duration/correlation violations.
- ubu-store: 88 tests, including admission rejection and accepted boundary cases.
- ubu-orchestrator: 75 tests, including all four advisory controller tests.

Commands: `CARGO_NET_OFFLINE=true cargo test` in core/store and
`CARGO_NET_OFFLINE=true cargo test --locked` in orchestrator. The entire
`tests/advisory_controller.rs` file is unchanged from `0d045e5`, verified with
`git diff --exit-code 0d045e5 -- tests/advisory_controller.rs`.

`cargo clippy --all-targets` completed in all three repos (orchestrator used
`--locked`). Existing warnings remain in core and orchestrator. Repository-wide
`cargo fmt --check` reports pre-existing formatting differences, including core's
`src/store/mutation_envelope.rs`, store's `src/api/mod.rs`, and orchestrator's
`src/config.rs`. Unrelated formatting was not swept into this type-alignment work.

## Bridges

Removed:

- `src/services/advisory_service.rs`: envelope and AdvisoryCandidate JSON
  round trips to the older store core. The writer now accepts the original
  envelope and candidate directly. The candidate idempotency key is cloned
  rather than reparsed.
- `src/state.rs`: DeviceId and actor UbuId string/reparse conversions in
  `envelope_with_key`; both now use the registered values directly.
- `src/services/advisory_service.rs` and `src/state.rs`: conversion of core
  errors into internal-error strings; the existing typed AppError::Core path
  now accepts them. This is an intentional consequence of removing type skew.
- All imports of the older core alias, including existing test imports.
  Planning's duration/correlation imports now point directly to core after
  removal of the store-local module.

Kept:

- `src/services/import_service.rs`: Task and ExternalReference JSON conversions
  from the pinned GitHub adapter's older core into the current core.
- `src/services/projection_service.rs`: authority conversion for the GitHub
  adapter's `apply_managed_label_write` API.
- `src/services/planning_service.rs`: legacy KernelPlan JSON read and kernel
  plan/request/duration/correlation projections. These serve the kernel/wire
  contract, not two definitions of the store's core types.
- Serialization of database payloads and HTTP response bodies remains ordinary
  persistence/wire encoding, not a bridge between different core revisions.

## Store test assertions and literal readings

In `ubu-store/tests/task_duration_estimate.rs`, the existing
`rejects_invalid_three_point_duration_ordering` retains its message assertion
and now additionally asserts the core NotStrictlyIncreasing violation. The
three existing accepted-input round-trip tests are unchanged.

Added assertions cover ZeroSeconds; both equality and greater-than failures at
each duration ordering boundary; StrengthOutOfRange on each side; DuplicateName;
and unknown fields in both duration variants and the correlation object. Accepted
boundary cases include seconds=1/u64::MAX, min=0, strengths=0/1, empty groups,
and a single empty group name (previously accepted). Failed admission must leave
no current object row.

Unknown-field failures remain StoreError::Json: serde's deny_unknown_fields
rejects them while deserializing the core types, before semantic validation.
There is no fabricated core violation for a serde error. Semantic errors now
propagate as StoreError::Core, as requested. Core's non-exhaustive
CorrelationGroupViolation distinguishes StrengthOutOfRange and DuplicateName;
the duration violation distinguishes ZeroSeconds and NotStrictlyIncreasing.
The inclusive range check also rejects NaN and infinities, preserving the
previous acceptance rules. No schema or data migration is needed.

E changes only this verification record; D contains the type/pin changes.

## F: committed dependency graph

Pushed in dependency order:

- Core: `cef80fe0ea68bf49413b6e75673dcd41274c9d75`.
- Store: `51a964af132083d1123b5baed0140e5945aa3ec2`.

Before D, Cargo listed four core sources: cf162b2, db37776, 139ce98, 2444b76.
After D, `cargo tree -i ubu_core` reports ambiguity and lists exactly three
source-qualified package ids. Running `cargo tree --locked -i` separately for
each id gives the following output. No db37776, path source, or alias remains.

```text
ubu_core v0.1.0 (https://github.com/UbU-project/ubu-core?rev=cef80fe0ea68bf49413b6e75673dcd41274c9d75#cef80fe0)
├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
└── ubu_store v0.1.0 (https://github.com/UbU-project/ubu-store?rev=51a964af132083d1123b5baed0140e5945aa3ec2#51a964af)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
ubu_core v0.1.0 (https://github.com/UbU-project/ubu-core?rev=139ce98a2dce40f8e73f472344d5cc4a8eb59db1#139ce98a)
├── ubu_planning_core v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=62c6d7a2f71078d141f7a4b9c91bd78193ee3314#62c6d7a2)
│   ├── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
│   └── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=62c6d7a2f71078d141f7a4b9c91bd78193ee3314#62c6d7a2)
│       └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
└── ubu_planning_cpu v0.1.0 (https://github.com/UbU-project/ubu-planning-kernel?rev=62c6d7a2f71078d141f7a4b9c91bd78193ee3314#62c6d7a2) (*)
ubu_core v0.1.0 (https://github.com/UbU-project/ubu-core?rev=2444b76e8eed75040e5a5791d1dce2579f0fe006#2444b76e)
└── ubu_github_adapter v0.1.0 (https://github.com/UbU-project/ubu-github-adapter?rev=4dce0887f0929584654e032992ed10fe5281a5cc#4dce0887)
    └── ubu_orchestrator v0.1.0 (/home/sean/ubu-phase1b/ubu-orchestrator)
```
