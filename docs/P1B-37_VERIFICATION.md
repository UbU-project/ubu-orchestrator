# P1B-37 verification

Baseline: `054d0d7829484e6c39da03d6923daf26d429d670`, the final P1B-37a
orchestrator revision. Its working tree and all eleven sibling repositories
were clean. Work is on `p1b-37-decomposition`; only the orchestrator repository
changes.

## Commit sequence

| Section | Commit | Change |
| --- | --- | --- |
| A | `8aff928` | Extract preparation without admitting a Task. |
| B | `2caa0f6` | Atomic decomposition, undo and Container listing. |
| C | `578bb56` | Compile eligible segment suffixes. |
| D | `4230f65` | Expand placed and unplaced carriers. |
| E | `76e61c7` | Scatter guard and regenerated OpenAPI. |
| F | `93b1b62` | Decomposition documentation and verbatim limits. |
| G | This commit | Fifteen regression tests, review corrections and verification. |

Sections A–E each passed the full existing suite: 304 passed, zero failed or
ignored. F changed documentation only. The final suite reports **319 passed,
zero failed, zero ignored**. Exactly fifteen new tests live in
`tests/decomposition.rs`.

The existing routine guard test was updated: its four legacy actions retain
`use_recorded_action`; decomposition now sends the required request and expects
`decompose_routine_occurrence_unsupported`. No existing test was removed.

## Gates

- `cargo test --locked --offline`: 319 passing tests.
- `cargo clippy --locked --offline --all-targets --message-format=json`:
  **10 → 9** distinct warnings, with no new warning category/location.
  Both measurements were taken in this execution on the orchestrator checkout.
  Counts deduplicate compiler warnings by code, message and primary file,
  line and column; repeated library/library-test emissions count once.
- `rustfmt --check --edition 2021 --config skip_children=true` passes for
  every created or edited Rust file. Pre-existing formatting elsewhere was
  left alone.
- OpenAPI regenerated with `cargo run --locked --offline --example generate_openapi`;
  all three routes are present.
- Every Cargo invocation used the existing dependency graph. Cargo.toml is
  unchanged, and Cargo.lock is byte-identical to the baseline, SHA-256
  `e7a0ecf2a14949e5d106ffc3d744605225c56c6ca9d049ff5e698790c6641a15`.
- All test commands ran under the seccomp wrapper that denies AF_INET and
  AF_INET6 socket creation, including spawned subprocesses. No test can
  reach the Internet; fixtures and stores are synthetic.
- The eleven sibling repositories retain their initial heads and clean trees.

The unchanged dependency pins are core `c77c0a2d1c3e206b8c18c023bb8b4ee0d06eb2d0`,
store `4066b8184403799fee031eb0660ee1bc12b3c1e8`, adapter
`4c7e3b6d31008a3bd64f28c303b8aa01bca3e2a0`, and both planning packages at
`84b6d0d9b621ca9a9df10034a8f8dc660baaca5d`. The store pin is P1B-37a's
functional revision, preceding its documentation-only follow-ups. No kernel
contract changed and no path dependency or patch table was added.

## Evidence

The targeted test run emitted:

```text
EVIDENCE[g1] before=(1, 0, 1) after=(5, 1, 7) batch_writes=6
EVIDENCE[g6] status=409 unchanged=true
EVIDENCE[g5] kernel_duration=ShiftedLognormalP95 { min_seconds: 701, mode_seconds: 902, p95_seconds: 1503 }
EVIDENCE[g15] compiled_members_unplaced=3 individual_members_placed=3
```

Counts are objects, Logs and mutation receipts. Six writes insert four objects
and one Log, update the origin, and add six receipts.

The three-step checklist plans Buy hinges at 09:00–09:30, Remove old hinges at
09:30–10:00, and Hang the gate at 10:00–10:30, with unrelated errands and a lunch
commitment in the same horizon. No scatter diagnostic appears. Two-segment
coverage proves a separate commitment can fall between contiguous pairs.

Fixed and mixed models are asserted on the request compiled for the kernel.
Expansion retains each child's category and ends at exactly the carrier end.
Tests also verify intersected windows, earliest deadlines, correlation union
with maximum shared strength, and a segment's maximum member value even when
its carrier has lower value.

All eight admission diagnostics are exercised without new writes. Completed
prefixes compile as suffixes; a missing middle child leaves the segment
uncompiled and produces its named diagnostic. An external dependency on a
removed member is rewritten and placed after the whole unit.

Undo preserves the original moot row, creates a fresh Task ID, supersedes the
Container, and reports completed children without changing their payloads or
versions. The restored Task is in the next Plan and the mooted children are
absent. Listing derives completion and reflects supersession.

## Interpretations and review corrections

There are no disagreements with the sixteen judgment calls.

- The operator explicitly approved one-based `child:N` proposed-sibling
  references. These resolve to new canonical handles before admission;
  ordinary Task IDs retain their existing meaning.
- Positions in list responses and segment indices are zero-based. Split
  points are exclusive boundaries, matching `Container::segments()`.
- The existing context type is `StorePlanningRequest`, the ticket's
  `PlanContext` equivalent. It and `DirectPlacements` carry compiled metadata
  through both candidate and canonical/repair expansion paths.
- The carrier's legacy `duration` sums the legacy member durations. Its typed
  duration model sums the actual Fixed seconds or stochastic components and
  drives placement and exact expansion.
- Preference layering still considers removed members so compilation cannot
  discard a valuable child's protection against omission.
- Original fields are captured in the decomposition Log. Undo uses that
  snapshot and copies only editable Task fields through ordinary preparation.
  Timing bounds that have expired or no longer fit the restored duration are
  omitted. A regression covers a shrinking allowed window and deadline while
  retaining the original Objective reference and duration.
- Direct non-capacity entries also rewrite dependencies on removed members.
- The decomposition Log checks the expected origin version before admitting
  anything; the final origin update checks it again within the same batch.
  Undo checks all observed child versions, including completed children.
- The scatter guard is a diagnostic; it does not change the explicitly
  deferred repair behavior.

All nine known limits are preserved verbatim in [DECOMPOSITION.md](DECOMPOSITION.md).
Full gate logs, the original repository snapshot and warning diagnostics are
retained locally in `../.p1b-37-amended-results/`.
