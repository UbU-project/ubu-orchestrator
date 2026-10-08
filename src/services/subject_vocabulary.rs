//! Operator-authored provisional subjects; governed roots are never Setting rows.
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    errors::{AppError, Result},
    state::AppState,
};
use serde_json::Value;
use crate::api::setting::SubjectReferenceCounts;

pub const GOVERNED: [&str; 5] = ["operator", "project", "github", "affect", "relationship"];
pub const PREFIX: &str = "universe.subject.";
pub const MAX_ROOT_BYTES: usize = 64;
pub const ROOT_SHAPE_MESSAGE: &str = "A subject must be lowercase ASCII snake_case, start with a letter, contain no dots, and be at most 64 characters.";

pub fn snake_case(name: &str) -> bool {
    name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && name.split('_').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

pub fn validate_root(root: &str) -> Result<()> {
    if [
        "facts",
        "numeric_values",
        "set_memberships",
        "event_markers",
        "affect",
    ]
    .contains(&root)
    {
        return Err(AppError::bad_request_diagnostic(
            "subject_reserved",
            format!("Subject `{root}` is reserved and cannot be minted."),
        ));
    }
    if GOVERNED.contains(&root) {
        return Err(AppError::bad_request_diagnostic(
            "subject_governed",
            format!("Subject `{root}` is governed and cannot be minted or retired."),
        ));
    }
    if root.len() > MAX_ROOT_BYTES || !snake_case(root) {
        return Err(AppError::bad_request_diagnostic(
            "subject_invalid",
            ROOT_SHAPE_MESSAGE,
        ));
    }
    Ok(())
}

pub async fn effective(state: &AppState) -> Result<BTreeSet<String>> {
    let mut roots: BTreeSet<String> = GOVERNED.into_iter().map(str::to_owned).collect();
    for row in super::setting_authoring::settings(state.inner().store.pool()).await? {
        let payload: Value = serde_json::from_str(&row.payload_json)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if payload["value"] != true {
            continue;
        }
        if let Some(root) = payload["name"]
            .as_str()
            .and_then(|name| name.strip_prefix(PREFIX))
        {
            if validate_root(root).is_ok() {
                roots.insert(root.to_owned());
            }
        }
    }
    Ok(roots)
}

/// Count all current object rows, including inactive Tasks and non-latest
/// UniverseStates. History/Logs are outside D0291's three named key spaces.
/// Task targets are nested JSON, so this is one full payload scan, not an
/// indexed lookup or one scan per root. No payload or target enters diagnostics.
pub async fn reference_counts(
    state: &AppState,
) -> Result<BTreeMap<String, SubjectReferenceCounts>> {
    let mut connection = state.inner().store.pool().acquire().await
        .map_err(|_| AppError::Internal("Could not count subject references in stored objects".into()))?;
    reference_counts_on(&mut connection).await
}

pub(crate) async fn reference_counts_on(
    connection: &mut sqlx::SqliteConnection,
) -> Result<BTreeMap<String, SubjectReferenceCounts>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT object_type, payload_json FROM objects WHERE object_type IN ('UniverseState', 'Task')",
    )
    .fetch_all(connection)
    .await
    .map_err(|_| AppError::Internal("Could not count subject references in stored objects".into()))?;
    let mut counts = BTreeMap::new();
    for (kind, text) in rows {
        let payload: Value = serde_json::from_str(&text).map_err(|_| {
            AppError::Internal("Could not count subject references in a stored payload".into())
        })?;
        count_payload(&kind, &payload, &mut counts)?;
    }
    Ok(counts)
}

fn target_root(target: &str) -> Option<&str> {
    let mut parts = target.split('.');
    matches!(parts.next()?, "facts" | "numeric_values" | "set_memberships" | "event_markers")
        .then(|| parts.next()).flatten().filter(|root| !root.is_empty())
}

fn increment(value: &mut u64) -> Result<()> {
    *value = value.checked_add(1)
        .ok_or_else(|| AppError::Internal("Subject reference count exceeds the supported range".into()))?;
    Ok(())
}

fn count_payload(
    kind: &str,
    payload: &Value,
    counts: &mut BTreeMap<String, SubjectReferenceCounts>,
) -> Result<()> {
    if !payload.is_object() {
        return Err(AppError::Internal("Could not count subject references in a stored payload".into()));
    }
    if kind == "UniverseState" {
        for field in ["facts", "numeric_values", "set_memberships", "event_markers", "fact_provenance"] {
            let Some(value) = payload.get(field) else { continue };
            let map = value.as_object().ok_or_else(|| {
                AppError::Internal("Could not count subject references in a stored collection".into())
            })?;
            for key in map.keys() {
                let root = if field == "fact_provenance" { target_root(key) }
                    else { key.split('.').next().filter(|root| !root.is_empty()) };
                if let Some(root) = root {
                    let entry = counts.entry(root.to_owned()).or_default();
                    if field == "fact_provenance" { increment(&mut entry.fact_provenance_keys)?; }
                    else { increment(&mut entry.universe_state_keys)?; }
                }
            }
        }
    } else if kind == "Task" {
        let mut pending: Vec<&Value> = payload.get("preconditions").into_iter().collect();
        while let Some(node) = pending.pop() {
            if let Some(root) = node.get("target").and_then(Value::as_str).and_then(target_root) {
                increment(&mut counts.entry(root.to_owned()).or_default().task_precondition_targets)?;
            }
            // Walk only condition groups, never expected values, effects or
            // arbitrary target-looking text in descriptions/candidate payloads.
            for field in ["all_of", "any_of"] {
                if let Some(children) = node.get(field).and_then(Value::as_array) {
                    pending.extend(children);
                }
            }
        }
    }
    Ok(())
}

/// New writes are grammatical; reads/evaluation and destructive legacy operations
/// keep core's existing target semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRefusal {
    Length,
    Grammar,
    Subject,
}
impl std::fmt::Display for TargetRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Length => "the target name exceeds 128 bytes",
            Self::Grammar => "the target requires a collection, subject, optional ASCII entity path and lowercase snake_case predicate",
            Self::Subject => "the subject is outside the effective vocabulary; mint it explicitly in UniverseState's Subjects list",
        })
    }
}
pub fn validate_target(
    target: &str,
    subjects: &BTreeSet<String>,
) -> std::result::Result<(), TargetRefusal> {
    if target.len() > super::precondition_advisor::MAX_TARGET_BYTES {
        return Err(TargetRefusal::Length);
    }
    let parts: Vec<_> = target.split('.').collect();
    if parts.len() < 3
        || ![
            "facts",
            "numeric_values",
            "set_memberships",
            "event_markers",
        ]
        .contains(&parts[0])
        || !snake_case(parts.last().unwrap())
        || parts[1..parts.len() - 1].iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
    {
        return Err(TargetRefusal::Grammar);
    }
    if !subjects.contains(parts[1]) {
        return Err(TargetRefusal::Subject);
    }
    Ok(())
}
pub fn target_pattern(subjects: &BTreeSet<String>) -> String {
    format!(
        r"^(facts|numeric_values)\.({})\.([A-Za-z0-9_-]+\.)*[a-z][a-z0-9]*(_[a-z0-9]+)*$",
        subjects.iter().cloned().collect::<Vec<_>>().join("|")
    )
}
