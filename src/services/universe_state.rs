//! The current `UniverseState`, read and edited by the operator.
//!
//! An edit is a list of `ubu-core` `UniverseMutation`s. They are checked and
//! applied by the same two functions a completed Task's effects go through.
//! This manual mutation route additionally checks first key segments; its
//! target grammar must hold for clients with no screen. See `docs/UNIVERSE_STATE.md`.

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
    validate_edit(mutations, mode)?;

    // A completed Task's effects are written under this lock too.
    let _action = state.inner().task_action_lock.lock().await;
    let pool = state.inner().store.pool();
    let subjects = super::subject_vocabulary::effective(state).await?;
    validate_write_targets(mutations, &subjects)?;
    let now = state.planning_now();
    let current = planning_service::read_current_universe_state(pool).await?;
    let base = match &current {
        Some((current, _)) => current.clone(),
        None => UniverseState::new(now, "empty UniverseState seeded by an operator edit"),
    };
    let next = prepare_edit(&base, mutations, mode, now)?;

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
                [(base.id.clone(), VersionRef::Absent)]
                    .into_iter()
                    .collect(),
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

fn validate_edit(mutations: &[UniverseMutation], mode: InstanceMode) -> Result<()> {
    if mutations.is_empty() {
        return Err(AppError::bad_request_diagnostic(
            "universe_mutations_empty",
            "mutations must list at least one mutation",
        ));
    }
    validate_mutations_for_mode(mode, mutations).map_err(|error| {
        AppError::bad_request_diagnostic("universe_mutation_mode_invalid", error.to_string())
    })?;
    validate_write_namespaces(mutations)?;

    Ok(())
}

fn validate_write_targets(mutations: &[UniverseMutation], subjects: &std::collections::BTreeSet<String>) -> Result<()> {
    use super::subject_vocabulary::{validate_target, TargetRefusal};
    for mutation in mutations {
        if !matches!(mutation.operation.as_str(), "set_fact" | "set_numeric" | "increment_numeric" | "decrement_numeric" | "add_membership" | "append_event_marker") { continue; }
        validate_write_namespaces(std::slice::from_ref(mutation))?;
        // Preserve core's established malformed-target diagnostics. The new
        // authoring checks govern targets core can already parse.
        if mutation.target.split('.').any(str::is_empty) || !matches!(mutation.target.split('.').next(), Some("facts" | "numeric_values" | "set_memberships" | "event_markers")) { continue; }
        if let Err(reason) = validate_target(&mutation.target, subjects) {
            let (code, message) = match reason {
                TargetRefusal::Subject => {
                    let subject = mutation.target.split('.').nth(1).unwrap_or_default();
                    ("universe_target_subject_unknown", format!("Subject `{subject}` is not in the effective vocabulary. Mint it explicitly in UniverseState's Subjects list before writing `{}`.", mutation.target))
                }
                _ => ("universe_target_grammar_invalid", format!("Target `{}` needs a subject and a predicate: <collection>.<subject>[.<entity-path>].<predicate>, with a lowercase snake_case predicate, ASCII entity segments and at most 128 characters.", mutation.target)),
            };
            return Err(AppError::bad_request_diagnostic(code, message));
        }
    }
    Ok(())
}

/// Shared operator mutation preparation: the screen and target admission use
/// identical mode, namespace and core mutation checks before either writer runs.
fn prepare_edit(
    base: &UniverseState,
    mutations: &[UniverseMutation],
    mode: InstanceMode,
    now: ubu_core::UbuTimestamp,
) -> Result<UniverseState> {
    validate_edit(mutations, mode)?;
    apply_universe_mutations(base, mutations, now).map_err(|error| {
        AppError::bad_request_diagnostic("universe_mutation_invalid", error.to_string())
    })
}

/// Prepare an operator assertion for the existing atomic candidate writer.
/// The caller holds task_action_lock through the write. Creating the complete
/// state atomically avoids leaving an empty seed if candidate admission fails.
pub(crate) async fn prepare_target_admission(
    state: &AppState,
    mutation: UniverseMutation,
) -> Result<(NewObjectRecord, VersionRef)> {
    let now = state.planning_now();
    let current = planning_service::read_current_universe_state(state.inner().store.pool()).await?;
    let base = current
        .as_ref()
        .map(|(world, _)| world.clone())
        .unwrap_or_else(|| {
            UniverseState::new(now, "empty UniverseState seeded by an operator edit")
        });
    let subjects = super::subject_vocabulary::effective(state).await?;
    let known = super::precondition_advisor::targets(&base);
    super::vocabulary::validate_name(&mutation.target, &known, &subjects).map_err(|reason| {
        AppError::bad_request_diagnostic("vocabulary_admission_refused", reason.to_string())
    })?;
    let next = prepare_edit(
        &base,
        &[mutation],
        crate::instance_mode::MVP_INSTANCE_MODE,
        now,
    )?;
    let existing = if current.is_some() {
        queries::get_current_state(state.inner().store.pool(), base.id.as_str()).await?
    } else {
        None
    };
    let observation = match current {
        Some((_, v)) => VersionRef::Version(
            u64::try_from(v)
                .map_err(|_| AppError::Internal("invalid UniverseState version".into()))?,
        ),
        None => VersionRef::Absent,
    };
    let mut payload = serde_json::to_value(&next).map_err(|e| AppError::Internal(e.to_string()))?;
    payload["schema_version"] = match &existing {
        Some(row) => serde_json::from_str::<serde_json::Value>(&row.payload_json)
            .map_err(|e| AppError::Internal(e.to_string()))?["schema_version"]
            .clone(),
        None => json!("core/universe-state/0.1"),
    };
    payload["provenance"] = json!({"created_at":now,"authority_source":AuthoritySource::User});
    Ok((
        NewObjectRecord {
            id: next.id.to_string(),
            object_type: ObjectType::UniverseState.as_str().into(),
            version: existing.as_ref().map_or(1, |r| r.version),
            status: existing
                .as_ref()
                .map_or_else(|| "active".into(), |r| r.status.clone()),
            compartment_label: existing.as_ref().map_or_else(
                || OPERATOR_SEED_COMPARTMENT.into(),
                |r| r.compartment_label.clone(),
            ),
            payload,
            created_at: existing.map_or_else(|| now.to_string(), |r| r.created_at),
            updated_at: now.to_string(),
        },
        observation,
    ))
}

pub(crate) fn reserved_key_segment(target: &str) -> Option<&str> {
    let segment = target.split_once('.')?.1.split('.').next()?;
    matches!(
        segment,
        "facts" | "numeric_values" | "set_memberships" | "event_markers" | "affect"
    )
    .then_some(segment)
}

/// Only the first key segment names the namespace. Cleanup remains available
/// for legacy keys; Task effects retain the core mutation contract.
fn validate_write_namespaces(mutations: &[UniverseMutation]) -> Result<()> {
    for mutation in mutations {
        if !matches!(
            mutation.operation.as_str(),
            "set_fact"
                | "set_numeric"
                | "increment_numeric"
                | "decrement_numeric"
                | "add_membership"
                | "append_event_marker"
        ) {
            continue;
        }
        let Some(segment) = reserved_key_segment(&mutation.target) else {
            continue;
        };
        let message = match segment {
            "facts" | "numeric_values" | "set_memberships" | "event_markers" => format!(
                "Key segment `{segment}` names a collection; the collection comes from the panel, not the key. The target would be `{}`.",
                mutation.target
            ),
            "affect" => format!(
                "Key segment `affect` is reserved for intrinsic affect, which organization_mode and worker_mode refuse. The target would be `{}`; the collection comes from the panel, not the key.",
                mutation.target
            ),
            _ => continue,
        };
        return Err(AppError::bad_request_diagnostic(
            "universe_target_namespace_invalid",
            message,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;

    fn mutation(
        operation: &str,
        target: &str,
        payload: Option<serde_json::Value>,
    ) -> UniverseMutation {
        UniverseMutation {
            operation: operation.to_owned(),
            target: target.to_owned(),
            payload,
            provenance_kind: None,
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
        for root in ["kettle", "shelf"] { super::super::setting_authoring::put(&state, &format!("universe.subject.{root}"), json!(true)).await.unwrap(); }
        let (returned, version) = apply(
            &state,
            &[
                mutation("set_fact", "facts.kettle.descaled", Some(json!(true))),
                mutation(
                    "increment_numeric",
                    "numeric_values.shelf.jars",
                    Some(json!(3)),
                ),
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
        for root in ["kettle", "shelf"] { super::super::setting_authoring::put(&state, &format!("universe.subject.{root}"), json!(true)).await.unwrap(); }
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
            assert!(
                message.contains("numeric_values.affect.energy"),
                "{message}"
            );
        }
        assert_eq!(stored_rows(&state).await, 0);

        // The manual route has a narrower namespace grammar in user_mode too.
        let refused = apply(&state, &affect, InstanceMode::UserMode)
            .await
            .unwrap_err();
        let AppError::Diagnostic { code, .. } = refused else {
            panic!("expected a diagnostic, got {refused:?}");
        };
        assert_eq!(code, "universe_target_namespace_invalid");
        assert_eq!(stored_rows(&state).await, 0);
    }

    #[tokio::test]
    async fn a_first_edit_seeds_a_user_capture_row_and_later_edits_keep_its_label() {
        let state = AppState::in_memory(ServerConfig::from_env()).await.unwrap();
        for root in ["kettle", "shelf"] { super::super::setting_authoring::put(&state, &format!("universe.subject.{root}"), json!(true)).await.unwrap(); }
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
