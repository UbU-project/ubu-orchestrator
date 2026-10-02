# Changelog

## Unreleased

- P1B-59 D: A measured number is a first-class fact.
  - `PATCH /universe-state` accepts `set_numeric` and `clear_numeric`, and an optional `provenance_kind` on each mutation. `GET /universe-state` and the edit's answer carry `fact_provenance`. A mutation may no longer carry `note`.
  - A Task's `preconditions` may use `at_least`, `at_most`, `greater_than` and `less_than`. Nothing in the planner changed for this: it calls `ubu-core`'s evaluator.
  - **Bootstrap's keys no longer repeat their collection, and this closes a mode-validation hole.** `POST /bootstrap/seed` stored `facts.operator.work_style` inside `facts`, so the target was `facts.facts.operator.work_style`. `ubu-core`'s `is_intrinsic_affect_target` reads the second dotted segment as the namespace. Under that convention an intrinsic-affect fact would be targeted `facts.facts.affect.x`, its namespace would read as `facts`, and the `organization_mode` and `worker_mode` guards would not fire for it. The keys are now `operator.work_style`, `operator.attention_preference`, `project.repository`, `project.objective` and `operator.planning_horizon_days`. There is no migration: a store bootstrapped before this keeps its doubled keys.
- P1B-59 C: `ubu_core`, `ubu_store`, `ubu_github_adapter` and both planning crates move together to the revisions that carry the new `ubu_core`.

## 0.1.0

- Initial public scaffold for the UbU Phase 1 local orchestrator.
