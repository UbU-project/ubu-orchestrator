//! Operator-authored provisional subjects; governed roots are never Setting rows.
use std::collections::BTreeSet;

use crate::{
    errors::{AppError, Result},
    state::AppState,
};
use serde_json::Value;

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
