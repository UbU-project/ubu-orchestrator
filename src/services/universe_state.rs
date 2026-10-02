//! The current `UniverseState`, read and edited by the operator.
//!
//! An edit is a list of `ubu-core` `UniverseMutation`s. They are checked and
//! applied by the same two functions a completed Task's effects go through, so
//! the operator's edits and a Task's effects share one vocabulary and one
//! validation path. See `docs/UNIVERSE_STATE.md`.

use serde_json::json;
use ubu_core::core::{
    apply_universe_mutations, validate_mutations_for_mode, InstanceMode, UniverseMutation,
    UniverseState,
};
use ubu_core::id_registry::ObjectType;
use ubu_core::{AuthoritySource, VersionRef};
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::queries;

use crate::errors::{AppError, Result};
use crate::services::planning_service;
use crate::state::AppState;

/// The label of a `UniverseState` row the operator's own edit creates. An edit
/// to an existing row keeps that row's label, whatever it is.
const OPERATOR_SEED_COMPARTMENT: &str = "user-capture";

/// The current state and its stored version. The version is `None` when the
/// store holds no `UniverseState`, and the state is then the empty one the
/// planner evaluates preconditions against.
pub async fn read(state: &AppState) -> Result<(UniverseState, Option<i64>)> {
    Ok(
        match planning_service::read_current_universe_state(state.inner().store.pool()).await? {
            Some((current, version)) => (current, Some(version)),
            None => (planning_service::synthesized_universe_state(), None),
        },
    )
}

/// Apply the operator's mutations to the current state and store the result as
/// its next version. Every mutation is checked before anything is written, so a
/// refusal leaves the store as it was.
pub async fn apply(
    state: &AppState,
    mutations: &[UniverseMutation],
    mode: InstanceMode,
) -> Result<(UniverseState, i64)> {
    if mutations.is_empty() {
        return Err(AppError::bad_request_diagnostic(
            "universe_mutations_empty",
            "mutations must list at least one mutation",
        ));
    }
    validate_mutations_for_mode(mode, mutations).map_err(|error| {
        AppError::bad_request_diagnostic("universe_mutation_mode_invalid", error.to_string())
    })?;

    // A completed Task's effects are written under this lock too.
    let _action = state.inner().task_action_lock.lock().await;
    let pool = state.inner().store.pool();
    let now = state.planning_now();
    let current = planning_service::read_current_universe_state(pool).await?;
    let base = match &current {
        Some((current, _)) => current.clone(),
        None => UniverseState::new(now, "empty UniverseState seeded by an operator edit"),
    };
    let next = apply_universe_mutations(&base, mutations).map_err(|error| {
        AppError::bad_request_diagnostic("universe_mutation_invalid", error.to_string())
    })?;

    let observed = match current {
        Some((_, version)) => version,
        None => {
            // The store updates an existing row and never creates one, so a store
            // with no state is given an empty one first, as a completion does.
            let mut payload =
                serde_json::to_value(&base).map_err(|e| AppError::Internal(e.to_string()))?;
            payload["schema_version"] = json!("core/universe-state/0.1");
            payload["provenance"] =
                json!({"created_at": now, "authority_source": AuthoritySource::User});
            let envelope = state.envelope_for(
                [(base.id.clone(), VersionRef::Absent)].into_iter().collect(),
                AuthoritySource::User,
                now,
            )?;
            let created = now.to_string();
            let seeded = queries::admit_object(
                pool,
                &envelope,
                NewObjectRecord {
                    id: base.id.to_string(),
                    object_type: ObjectType::UniverseState.as_str().into(),
                    version: 1,
                    status: "active".into(),
                    compartment_label: OPERATOR_SEED_COMPARTMENT.into(),
                    payload,
                    created_at: created.clone(),
                    updated_at: created,
                },
            )
            .await?;
            seeded.version
        }
    };

    let observed = u64::try_from(observed)
        .map_err(|e| AppError::Internal(format!("invalid stored object version: {e}")))?;
    let envelope = state.envelope_for(
        [(next.id.clone(), VersionRef::Version(observed))]
            .into_iter()
            .collect(),
        AuthoritySource::User,
        now,
    )?;
    let stored = queries::persist_universe_state(pool, &envelope, &next, AuthoritySource::User)
        .await
        .map_err(AppError::from)?;
    Ok((next, stored.version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;

    fn mutation(operation: &str, target: &str, payload: Option<serde_json::Value>) -> UniverseMutation {
        UniverseMutation {
            operation: operation.to_owned(),
            target: target.to_owned(),
            payload,
            note: None,
        }
    }

    async fn stored_rows(state: &AppState) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM objects WHERE object_type = 'UniverseState'")
            .fetch_one(state.inner().store.pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn the_planner_reads_the_state_a_write_returned() {
        let state = AppState::in_memory(ServerConfig::from_env()).await.unwrap();
        let (returned, version) = apply(
            &state,
            &[
                mutation("set_fact", "facts.kettle.descaled", Some(json!(true))),
                mutation("increment_numeric", "numeric_values.shelf.jars", Some(json!(3))),
            ],
            InstanceMode::UserMode,
        )
        .await
        .unwrap();

        let (planner_reads, planner_version) =
            planning_service::read_current_universe_state(state.inner().store.pool())
                .await
                .unwrap()
                .expect("a stored UniverseState");
        assert_eq!(planner_reads, returned);
        assert_eq!(planner_version, version);
        assert_eq!(planner_reads.facts["kettle.descaled"], json!(true));
        assert_eq!(planner_reads.numeric_values["shelf.jars"], 3.0);
    }

    #[tokio::test]
    async fn an_intrinsic_affect_target_is_refused_outside_user_mode_and_nothing_is_written() {
        let state = AppState::in_memory(ServerConfig::from_env()).await.unwrap();
        let affect = [mutation(
            "increment_numeric",
            "numeric_values.affect.energy",
            Some(json!(1.0)),
        )];
        for mode in [InstanceMode::OrganizationMode, InstanceMode::WorkerMode] {
            let refused = apply(&state, &affect, mode).await.unwrap_err();
            let AppError::Diagnostic { code, message, .. } = refused else {
                panic!("expected a diagnostic, got {refused:?}");
            };
            assert_eq!(code, "universe_mutation_mode_invalid");
            assert!(message.contains("numeric_values.affect.energy"), "{message}");
        }
        assert_eq!(stored_rows(&state).await, 0);

        // The same edit is the operator's to make in `user_mode`.
        let (written, _) = apply(&state, &affect, InstanceMode::UserMode).await.unwrap();
        assert_eq!(written.numeric_values["affect.energy"], 1.0);
    }

    #[tokio::test]
    async fn a_first_edit_seeds_a_user_capture_row_and_later_edits_keep_its_label() {
        let state = AppState::in_memory(ServerConfig::from_env()).await.unwrap();
        for value in [json!("first"), json!("second")] {
            apply(
                &state,
                &[mutation("set_fact", "facts.kettle.note", Some(value))],
                InstanceMode::UserMode,
            )
            .await
            .unwrap();
        }
        let labels: Vec<String> = sqlx::query_scalar(
            "SELECT compartment_label FROM objects WHERE object_type = 'UniverseState'",
        )
        .fetch_all(state.inner().store.pool())
        .await
        .unwrap();
        assert_eq!(labels, vec![OPERATOR_SEED_COMPARTMENT.to_owned()]);
    }
}
