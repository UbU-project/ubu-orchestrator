# P1B-13 Task mapping fields and constructor

## Implementation and dependency order

All nine sibling repositories started clean. Devshell update-all reported all
existing siblings up to date, then its unchanged DS-1 generator created local
patch configs. Implementation tests ran offline against local siblings.

Pushed in dependency order:

| Repository | Revision | Section |
| --- | --- | --- |
| ubu-schemas | d51d360f90d1ec602915d6db2a5cae10a9da2f01 | A |
| ubu-core | 59f04f470aa5696e6762b6b594f1a02918bc525b | B, C, D |
| ubu-store | 4ed3a0a97e4ede4a57d13bd05d69688f128e3755 | E |
| ubu-github-adapter | 9ab7f7d752d22a9701250d1107f403745b795c66 | F |
| ubu-planning-kernel | 9e8ea74a0c7f306dd7770829c3e9457d5475dfb5 | G |

Orchestrator H updates all five dependency entries and their committed lockfile
sources to these revisions. Its source and tests are byte-for-byte unchanged
from d9cc7fa, including the P1B-9 and P1B-12 assertions. The kernel changes only
Cargo.toml and Cargo.lock; its source, tests, fixtures and planning contract
version are unchanged. Each lettered section has a separate commit.

## Tests

- Schemas: `cargo run -p validate-fixtures` accepted 67 valid fixtures and rejected
  58 invalid fixtures. `cargo test --workspace` passed its one test.
- Core: 118 tests passed, including constructor defaults for every optional field,
  capacity default/false round trips, all three new fixtures, window equality and
  reverse ordering, timezone-equivalent instants, category case sensitivity,
  malformed shapes and independence of duration/window/capacity.
- Store: 95 tests passed. New tests verify typed core rejections, successful Static,
  non-capacity and categorized Tasks, retained wrapper fields, malformed field
  shapes, and atomic rejection of an invalid update without a ledger write.
- Adapter: all 23 tests passed with existing in-memory fakes.
- Kernel: all 35 tests passed both before the bump and after it, with identical
  named outcomes. Every skeleton, affect, scoring/selection, rollout and degraded
  rollout golden matched, as did every existing property/invariant check.
- Orchestrator: all 84 tests passed without changing source or test assertions.

All implementation commands used CARGO_NET_OFFLINE=true. Committed-source library
checks additionally passed for store, adapter and the full kernel workspace.

## I: downstream construction is compiler-enforced

In the actual adapter checkout, a temporary probe replaced only the task helper
with its pre-ticket literal from 4b31c8c. `cargo check --lib --offline` failed with:

```text
error[E0639]: cannot create non-exhaustive struct using struct expression
```

A second probe supplied all three new fields in that literal. It still failed
with E0639, proving rejection comes from non-exhaustiveness rather than missing
fields. A try/finally restored the constructor implementation byte-for-byte after
both checks. The subsequent committed-source adapter check passed.

`git grep -n -E '\bTask[[:space:]]*\{' -- '*.rs'` across every changed downstream
repository reports only these non-construction matches:

```text
ubu-store/src/admission.rs:129:    if object_type != ObjectType::Task {
ubu-github-adapter/src/candidate_mapping.rs:166:) -> Task {
```

The first is an ObjectType comparison; the second is a function return signature.
No Task struct construction remains outside ubu-core. Full probe compiler output
and grep evidence are retained in the sibling P1B-13-results directory.

## Literal readings and representation choices

- Schema `schemas/core/task.schema.json:141` and core `src/core/task.rs:77`: the
  new fields append after tags in category_tag, occupies_capacity, static_window
  order; both the public struct and custom deserialization wire follow that order.
- Core `src/core/task.rs:79` and `:234`: occupies_capacity defaults to true on
  deserialization and omits true on serialization. False remains explicit. This
  preserves existing fixture round trips without introducing compatibility work
  or changing planning output; backward compatibility was not required.
- Core `src/core/task.rs:274` and `:298`: Task::new accepts exactly the four required
  fields and never validates. Task::validate delegates to the existing lifecycle
  validator, which also checks validate_fields. The internal core literal remains.
- Core `src/core/task.rs:104` and `:113`: standalone StaticWindow::validate and
  validate_category_tag expose the same rules used by Task and store. Distinct
  errors are InvalidTaskStaticWindow and InvalidTaskCategoryTag
  (`src/errors.rs:132`, `:134`). Comparisons use timestamp instants, not string order.
- Core `src/core/task.rs:113`: category selection rejects empty strings as well as
  non-members, satisfying the schema's non-empty rule even when the standalone
  validator receives an empty tag. Matching is exact and case-sensitive.
- Core `src/core/task.rs:146` and `:336`: a present duration estimate still obeys
  its own validity rules, but is never compared to the window. No planner placement
  or capacity logic is changed. Static/non-capacity and Dynamic/non-capacity
  combinations are accepted.
- Store `src/admission.rs:143`: absent tags act as an empty list when checking a
  present category_tag. The store validates the field's string/array shapes and
  calls core's membership validator. It does not deserialize the entire Task or
  remove wrapper fields.
- Store `src/admission.rs:153`: a present occupies_capacity must deserialize as a
  boolean, aligning its shape with schema/core. An absent value stays absent in
  raw stored JSON and is interpreted as true by core; the store does not rewrite
  payloads to materialize defaults. This adds shape validation alongside the two
  explicitly requested cross-field checks.
- Store `src/admission.rs:157`: nested StaticWindow fields are decoded strictly,
  then core checks ordering. Malformed JSON shapes remain StoreError::Json;
  semantic violations propagate as StoreError::Core, like duration/correlation.
- Core `schemas-ref` gitlink and `README.md:21`: advanced the canonical fixture
  submodule to the pushed A revision. The four new fixtures were also copied
  byte-for-byte into placeholders, as requested. No cross-field-invalid schema
  fixture or custom JSON Schema extension was added for the new rules.
- Adapter `src/candidate_mapping.rs:167`: Task::new receives id/title/status/
  provenance; only description and moot_reason_code are subsequently assigned.
  No GitHub mapping is invented for the new fields, and CandidateObject is retained.
- Orchestrator `Cargo.toml:21`: "all five pins" means core, store, adapter and
  both kernel package entries. All manifests use the identical full core revision.
  Cargo.lock retains registry versions and committed git sources, not local paths.
- Verification sections I and J are separate documentation commits in the
  orchestrator; full logs and tree outputs live outside the repositories.

No new dependency, devshell change, reminders/after/lore field, or orchestrator
use of the new mapping fields was introduced. P1B-14 planning/projection work
remains out of scope.
