# Admission review contract

Every admitted value is open to advisory review. The advisory boundary makes
that safe: a review is a candidate, and admitting it is the operator's act.
A review carries the existing value, a nonblank model reason, and a replacement
if there is one. Removal is an explicit operation. A sound verdict is an
aggregate information diagnostic, never a candidate. Model reasons belong in
candidates and never in diagnostics.

A reviewer reads only what the operator can see: Task ID, description, existing
precondition in the screen's words, supported UniverseState target names, and a
prior operator rejection reason for that subject. It reads no fact values, Log
or other Task fields. Each model call sees one Task only; the run budget covers
all selected Tasks and sound verdicts aggregate into one diagnostic. Replacement follows P1B-61: strict shape, nonempty groups,
node/depth bounds, mode check, every leaf checked, then whole-tree evaluation.
Every proposed target must exist, even for `absent`. A missing fact can later be
authored, so a comparison is not blocked forever.

A dismissal is a snooze, never permanent. The subject suppression key is SHA-256
of canonical JSON containing Task reference, field (`preconditions`), and the
existing value. It excludes model wording and replacement. Editing the value
invalidates its hold immediately. The escalation key is Task and field only:
editing the value preserves escalation. Count dismissal events, including
repeated deferrals of one candidate, since the last admitted review of the slot.
Admission resets the count. No counter belongs on the Task. Event insertion
order disambiguates equal or backdated timestamps.

The default seed is **7 days**, doubling to a **365-day ceiling**:
**7, 14, 28, 56, 112, 224, 365, 365**. Settings
`advisory.review_seed_days` and `advisory.review_ceiling_days` are integers with
`1 <= seed <= ceiling <= 365`; reverting also validates the pair. The screen
preselects the escalated suggestion and offers shorter spans.
**The blocking-now cap overrides escalation, never the reverse:** at dismissal,
a false requirement caps the actual span at the seed, including a longer choice
made before facts changed. A shorter choice stays shorter. The screen explains
the cap. Actual span and return date commit atomically with the decision in the
immutable mutation ledger; restarts and later Settings edits do not rewrite it.

Admit checks the reviewed value and observed versions through ordinary atomic
admission. Defer holds the candidate. Reject records that the analysis is wrong,
with an optional operator reason, and holds the subject for the same interval.
An omitted reason is the neutral marker “No reason provided”, never passed to
the model as an operator-authored explanation. Renewed rejection refreshes the
current suppression record while immutable history retains prior decisions and
reasons. Hold diagnostics give a date. Nothing offers “never”.

Runs are on demand. Normal **Review preconditions** honours snoozes; explicit
**Review again now** bypasses them. Deferred candidates return through
`PolicyReviewInterval` after expiry or `UserRequest` on explicit reconsideration.
Core forbids `Rejected → Resurfaced`: expiry/bypass permits a fresh candidate.
An active equivalent review is reused. A deferred candidate cannot be admitted
until resurfaced. `AcceptedChangeToTargetOrDependencies` has its effect through
the subject key, without a lifecycle transition. `MateriallyNewEvidence` and
`ClarificationOrExternalReferenceArrival` remain unused; there is no scheduler.
The existing manual resurface route already used `UserRequest` before P1B-62.

Review starts with preconditions because they affect whether work can be placed.
Tags, categories, duration estimates and event colours are not reviewed yet.
Later reviewers follow this contract; a ticket changing it changes this file in
the same commit.
