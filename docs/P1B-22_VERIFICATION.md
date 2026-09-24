# P1B-22 verification

Baseline: `ubu-orchestrator` at `c2813193735f3754c3289efbd53f18078593ac1e`.
Branch: `p1b-22-static-containment`. Only this repository changes.

## Validation

- `cargo test --all-targets`: **176 passed**, up from 172. Sections A, B and C
  each passed all 172 existing tests before their individual commits; section D
  adds exactly four HTTP tests and passes all 176.
- `cargo clippy --all-targets`: **11 before, 10 after** on rustc 1.98.1 /
  clippy 0.1.98 (`48a229cea`, 2026-09-01). Count distinct emitted diagnostics,
  excluding Cargo's `generated N warnings` summaries and duplicate target
  passes. Separate locations with the same message count separately, including
  the two existing clone-on-Copy diagnostics. The only removed diagnostic is
  `this if statement can be collapsed` in the Static pairwise conflict clause.
  The other diagnostic-message counts are identical; no warning is added.
- The ticket's **13 -> 12** absolute counts did not reproduce; its required
  **decrease of exactly one** did. P1B-21 also recorded the eleven-warning
  baseline. No unrelated warning was fixed or manufactured to reach 12.
- `Cargo.lock` and `Cargo.toml` are byte-identical to the baseline. Lockfile
  SHA-256: `486369199237d3d21ea39523f58204be7a6287a24a4d67264c76c24fa19d3bbe`.
  Every git pin is unchanged and there are no new dependencies.
- Production changes are **20 added / 7 removed lines**, all in
  `src/services/planning_service.rs` (27 changed lines, under 40).
- Only `tests/static_containment.rs` was formatted with `rustfmt --edition 2021`;
  no repository-wide formatting was run. `git diff --check` passes.
- All eleven read-only sibling repositories retain their original revisions
  and clean trees. No OpenAPI, existing test, fixture, runtime configuration,
  compiler concurrency setting, or other repository was changed.

The four tests exercise `/planning/generate` with synthetic admitted Tasks.
Success cases also read `/calendar/current`, verifying true timestamps,
capacity flags, membership and placement. Test 1 checks a six-hour block, a
five-minute chore, and fixed-duration Dynamic work placed wholly outside the
block. Its subcases cover a shared start, a shared end, and the contained
Static-prerequisite exception. Tests 2 and 3 reject equal and partial overlaps,
both alone and inside a shared container. Test 4 checks three nested blocks,
reverse admission order, and the latest-end carrier tie-break for equal starts.
The successful groups emit exactly one diagnostic and no corresponding risk
finding. No test reads a real Quick UbU file.

## Verbatim messages

Test 1 (`contained_chore_keeps_its_window_and_dynamic_work_stays_outside`):

```text
1 Static Task happens during `task_018f3c8e9b2a7c4d8f1e2a3b4c5d7e01`; the whole span is busy and every one of them stays on the Calendar
```

Test 4 (`three_nested_blocks_share_one_outermost_carrier`):

```text
2 Static Tasks happen during `task_018f3c8e9b2a7c4d8f1e2a3b4c5d7e01`; the whole span is busy and every one of them stays on the Calendar
```

## Carrier consequence

Containment can now join plain Static members to an existing routine cluster.
A newly joined container starting earliest becomes the carrier under the
unchanged `(start, Reverse(end), id)` ordering. The kernel reserves the group's
whole union, inherits its external dependencies, and the response restores each
member's true window. A newly connected containing commitment can therefore
change an existing cluster's carrier.

**No existing routine test's carrier moved.** Inspection of the synthetic
routine fixtures and test windows shows no newly joined pair of plain Static
Tasks in those clusters, and the unchanged routine suite passes. The pinned
meeting already carried the afternoon chore; the late event already carried
the overlapping sleep span; Direct A still carries the directly admitted
three-occurrence cluster. A container directly overlapping a mandatory
occurrence already joined under the old rule. Thus section C's stated
consequence applies to newly connected plain members, not every container
that already surrounds a routine occurrence.

## Judgment calls and literal readings

No disagreement with the seven intended behavior choices: proper containment
only, whole-span reservation, reuse of `committed_clusters`, a plain diagnostic,
the prerequisite exception, retained capacity flags, and removal of exactly
the specified Clippy warning. The absolute Clippy baseline is the factual
discrepancy recorded above.

- Section B says both clauses gate on capacity. The source explicitly includes
  non-capacity Static prerequisites in clause (b); only the pairwise occupancy
  clause gates on both capacity flags. Applied the literal requested
  `else if !nested(window, other_window)` without adding a new gate. Transparent
  Tasks still bypass clustering, but a nested Static dependency involving one
  receives that same conflict exception.
- “Nothing else in the function changes” in C means retain union-find,
  ordering, dependency inheritance and reattachment, while adding C's explicitly
  requested diagnostic block to its sorted warnings vector.
- `N` counts members other than the carrier, matching the singular and
  three-block examples. Only groups without routine occurrences get this code;
  existing routine diagnostics and mandatory overlap exceptions remain intact.
- Shared edges count as proper containment when the other edge differs.
  Equal windows and partial overlaps remain pairwise conflicts even if a
  transitive group could otherwise connect both Tasks through a container.
- The under-40-line limit applies to production source, excluding D's expressly
  required new tests and documentation.

## Ticket-supplied real-snapshot measurements

These are measurements supplied in P1B-22 as motivation, **not runs performed
for this verification**. No private snapshot was read or committed.

| Supplied run | Supplied result |
| --- | --- |
| Before | `rejected`, 0 steps, 25 `static_task_collision` diagnostics |
| After | `rejected`, 0 steps, 3 collisions, 2 containment groups: 18 Tasks during one, 1 during the other |
| After moving the three stale 16:00 pins apart as the newest routine file already does | `ok`, 30 steps, 0 collisions |

The surviving three collisions are the equal-span trio and deliberately remain
collisions; this ticket does not hide them.

The fixed-duration limitation in `P1B-21_VERIFICATION.md` is unchanged:
coverage and `display_probability` still read **1.0** on fitting imported data
because every imported Task carries a point duration. Containment introduces
no duration uncertainty or change to those calculations.
