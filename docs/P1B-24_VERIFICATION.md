# P1B-24 verification

All twelve sibling working trees were clean at the ticket's expected revisions
before work. The seven changed repositories use `p1b-24-bounded-after`.

## Landing order and pins

| Repository | Pushed revision |
| --- | --- |
| ubu-schemas | `5c134ade8b5b3154b02dd418ce225f22bd3edd76` (A) |
| ubu-core | `e02267763f86750a40a84b3c47d1a8f0d173084b` (B) |
| quick-ubu, independent | `1492a0c29308c1bb25c789d6650ce605e3694f4f` (D) |
| ubu-store | `6fa8826cee7c75590e64f823c43911b2702dd0ca` (C) |
| ubu-planning-kernel | `6595bfe5c38cf62f56fe2a777330017d967d7283` (C) |
| ubu-github-adapter | `e47d5bf2e2c14d84320061cbf476570820059faf` (C) |
| ubu-orchestrator | `p1b-24-bounded-after`, the section J commit containing this report |

The three C consumers form one independent landing layer. Quick UbU landed
before the orchestrator as permitted. The orchestrator's immutable pre-J
revision is `bee96b41d5d9cd0a84c0a8e50b6b0b500c529ed2` (I); all production
behavior was complete at H, `2dafe67`. J adds only the six-test file and this
report. The final J SHA is reported in the final ticket results rather than
attempting to embed a commit's own hash into its contents.

Core's `schemas-ref` gitlink matches A exactly. Store, kernel and adapter each
pin B exactly. Orchestrator pins that same B revision plus the three exact C
revisions (both planning crates use the same kernel revision). Upstream branch
tips were confirmed pushed before the orchestrator fetched them. There are no
new dependencies, path dependency overrides, or `[patch]` tables in any commit;
existing intra-workspace paths in Quick UbU and the kernel are unchanged.

## Tests and Clippy

| Repository | Passing tests | Distinct Clippy warnings, before → after |
| --- | ---: | ---: |
| ubu-schemas | 1, plus 75 valid / 71 invalid fixtures | 0 → 0 |
| ubu-core | 129 | 2 → 2 |
| ubu-store | 100 | 0 → 0 |
| ubu-planning-kernel | 81 | 0 → 0 |
| ubu-github-adapter | 23 | 0 → 0 |
| quick-ubu | 444 | 16 → 16 |
| ubu-orchestrator | 192 | 10 → 10 |

`cargo run -p validate-fixtures` passes. Full suites ran with ordinary Cargo
concurrency: `cargo test --workspace` for schemas, C consumers and Quick UbU;
`cargo test --all-targets` plus doc tests for core and orchestrator. No compiler
or test-thread caps were introduced. Every lettered commit followed a green
full suite. Orchestrator E–I each passed 186 tests; J adds exactly six, reaching
192. Core adds one bounds-validation test; Quick UbU adds one round-trip test.

Clippy was measured with `cargo clippy --all-targets` on each actual checkout
before edits and after its changes, on rustc 1.98.1 / clippy 0.1.98. Counts exclude
Cargo's `generated N warnings` summaries and duplicate target passes; different
source locations count separately even when their messages match. The complete
before/after diagnostic-message multisets match in every repository. No warning
was added or fixed, and no previous ticket's quoted count was used.

`cargo run --example generate_openapi` succeeded; **OpenAPI is byte-unchanged**.
Only the new orchestrator test was formatted with rustfmt. Core remains
fmt-clean. Contrary to the ticket's baseline claim, store and Quick UbU already
failed `cargo fmt --all -- --check` before changes; no repository-wide formatting
was applied to them. Their unrelated formatting divergence was left alone.
`git diff --check` passes across all seven repositories. The five read-only
siblings retain their exact starting revisions and clean trees.

## Lockfiles: the ticket's conflicting instructions

The general workflow and verification request say six non-orchestrator lockfiles
must be byte-identical. Section C explicitly requires `cargo update -p ubu_core`
and the corresponding entry to move in each consumer lockfile. Both requirements
cannot hold at once. We followed the specific section C instruction, preserving
an honest buildable pin graph, and report the actual result rather than claiming
six unchanged lockfiles:

- Schemas, core and Quick UbU lockfiles are **byte-identical** to their baselines.
  Core's local lockfile is ignored by Git; it was compared on disk too.
- Store, kernel and adapter lockfiles each change **only the `ubu_core` git source**
  to B. Their manifests each change only that same pin.
- Orchestrator's lockfile changes **exactly five entries**, source revisions only:
  `ubu_core`, `ubu_store`, `ubu_planning_core`, `ubu_planning_cpu`,
  `ubu_github_adapter`.

Package lists, versions, checksums and dependency arrays are identical after
excluding those source fields. No added or removed packages and no update-all
operation occurred.

## Authorized store test exception

Section C assumed no consumer source file referenced `offset_seconds` and
required stopping if one needed a change. The first store suite failed in
`routine_admission_checks_each_cross_field_without_rejecting_wrapper_fields`:
`tests/routines.rs:47` still constructed an after edge with that removed field.
Its intended self-reference validation was never reached under the new type.

Work stopped and this was reported. The user explicitly authorized:

> Make that one-line test update and continue. Report this in the results.

The only store source change is that test's JSON key:
`"offset_seconds":0` → `"minimum_seconds":0`. No store production code changed.
The store's full suite then passed all 100 tests. Kernel and adapter remain
strictly manifest/lockfile repins with no source changes.

## Verbatim kernel-visible windows

Test 1 calls `planning_service::build_request_from_store` and prints:

```text
P1B24_UNBOUNDED {"end":"2026-09-24T23:59:00Z","start":"2026-09-24T21:01:00Z"}
P1B24_BOUNDED {"end":"2026-09-24T21:41:00Z","start":"2026-09-24T21:01:00Z"}
```

The successor lasts thirty minutes, its predecessor ends nominally at 21:01,
and the bounded edge permits a start within 600 seconds. The finish channel
therefore becomes 21:01 + 10 minutes + 30 minutes = 21:41.

Test 5 uses a declared latest finish of 21:55 and records the predecessor's
completion at 21:21, twenty minutes after its nominal end. It prints:

```text
P1B24_REALIZED_BEFORE {"end":"2026-09-24T21:41:00Z","start":"2026-09-24T21:01:00Z"}
P1B24_REALIZED_AFTER {"end":"2026-09-24T21:55:00Z","start":"2026-09-24T21:21:00Z"}
```

The realized finish calculation gives 22:01, clamped to declared 21:55, not the
stale lowered 21:41. Its second subcase completes early at 21:00:30 and checks a
21:40:30 ceiling, demonstrating that assignment moves it earlier too.

The remaining tests verify tightest bounds from two predecessors and duplicate
edge merging, maximum-specific versus pre-existing infeasibility, Static
placement retention despite a contradictory maximum, and HTTP import through
planning. Import rejects inverted and fractional edges with the specified
reasons while retaining the valid bounded edge. All inputs are synthetic.

## Quick UbU round trip

`command_tests::routine_import_then_snapshot_preserves_after_maximum` writes a
synthetic `routine.json`, runs `routine-import`, then `snapshot`, and reads the
result. The stored value is exactly:

```json
{"offset":[0,0],"maximum":[600,0]}
```

The test asserts both Duration arrays in the exported after item. Quick UbU's
scheduler is unchanged; it continues to ignore maximum. No real Quick UbU
snapshot or routine file was read or committed.

## Judgment calls and literal readings

No disagreement with the eight behavior choices: honest pins, start-relative
bounds, no kernel bounded-edge contract, finish-window lowering, tightest
maximum, assigned realized ceiling clamped to the declared range, a distinct
maximum-infeasibility diagnostic, and Quick UbU retaining `offset`.

Additional ambiguities were resolved as follows:

- E cannot compile against B's renamed field without compatibility edits. E
  therefore mechanically renames the field in the instantiator, importer and
  existing tests, before any ceiling behavior. F's triple tuple likewise needs
  simultaneous destructuring/assertion updates to keep that commit green.
- The mandatory minimum has no alias for the removed mainline name. Existing
  mainline declarations using `offset_seconds` need updated inputs; unchanged
  Quick UbU declarations remain supported through offset-to-minimum translation.
- Missing maximum is unbounded. Duplicate absent maxima do not erase a present
  bound. Arithmetic follows existing saturating timestamp arithmetic.
- Only matched same-date predecessors contribute nominal bounds, preserving
  existing unmatched-reference diagnostics. Realized ceilings aggregate the
  completed predecessors that supply actual ends in the existing loop.
- Duration in the ceiling calculation is the template's scalar declaration,
  as specified. P1B-23's later request-only observed estimate does not rewrite
  that declaration or recompute the lowered window.
- Invalid imported bounds skip the offending edge, following the existing
  per-edge validation chain; they do not discard the whole routine. An absent
  mainline maximum is omitted, not serialized as null.
- Existing horizon participation and declaration-based eligibility checks were
  left in place. This is the requested lowering slice, not a general rescheduler
  for occurrences already excluded before realized context is applied.
- The supplied probe describes roughly seven-and-a-half hours; its exact
  timestamps differ by 7 hours 40 minutes. Documentation includes the verbatim
  probe. Preconditions gate against current state, effects apply on completion,
  and ordinary explicit Task dependencies still exist alongside routine after.
- The lockfile and formatting baseline discrepancies, plus the user-authorized
  store-test exception, are recorded above.

## Known limits (verbatim)

1. **The ceiling does not travel with the predecessor inside a rollout.** It is an absolute window bound, fixed when the request is built. If the predecessor samples long, the successor's start is pushed past a ceiling that did not move with it, and the day reads less feasible than a true bounded edge would make it. This **overstates** risk, which is the safe direction, and it is small where it matters — the Tasks that need tight maxima are short ones. Fixing it means a bounded edge in the kernel contract and the `UBU-D0283` worker: judgment call 3.
2. **A start ceiling is carried as a finish bound.** `window.end = latest_start + duration` is exact only while the sampled duration equals the planned one. A `UBU-D0285` overrun of the successor itself can therefore read infeasible even though its start was inside the bound. There is no start-ceiling channel for Dynamic work to put it in.
3. **An occurrence dropped at instantiation is not `unplaced_tasks`.** Both infeasibility codes return `None` from `expand`, so the Task is never created and `UBU-D0289`'s partial-placement machinery never sees it — including for a mandatory routine, which simply vanishes from the day with a diagnostic. That is pre-existing behaviour of `routine_after_infeasible`, not introduced here, but a second code now reaches it.
4. **Quick UbU does not honour `maximum`.** It carries the field and plans as it always did. Until the switch, the two tools will disagree about any routine that declares one.

