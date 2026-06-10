# Contract

This crate owns the Phase 1 local orchestration contract between `ubu-ui`,
`ubu-store`, `ubu-github-adapter`, and `ubu-planning-kernel`.

## Authority

Planning validation and legitimization are authoritative in
`ubu_planning_core::validate_plan` and the core planning flow. The orchestrator
does not certify plans itself.

Projection approval uses the canonical `ubu_core::AuthoritySource` enum,
including `user` and `user_override`.

## Required flow

bootstrap -> import GitHub fixture -> admit candidates to store -> build
PlanningRequest -> call planner -> admit Plan and Calendar -> expose next action
-> accept user action -> append LogEntry -> recalculate -> generate
ProjectionPreview -> approve ProjectionPreview batch -> return ProjectionResult.
