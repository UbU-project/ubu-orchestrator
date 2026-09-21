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
