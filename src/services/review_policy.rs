//! Review snoozes are policy over immutable decision history, never Task counters.
use super::{
    precondition_advisor,
    precondition_review::{self, ReviewTask},
    setting_authoring,
};
use crate::{
    errors::{AppError, Result},
    state::AppState,
};
use serde::{Serialize, Deserialize};
use serde_json::{json, Value};
use ubu_core::core::{evaluate_universe_precondition, Task};
use ubu_core::{
    AdvisoryCandidate, AuthoritySource, CandidateLifecycleState, ResurfaceTrigger, UbuTimestamp,
};

fn internal(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(e.to_string())
}
fn after(now: UbuTimestamp, days: u64) -> String {
    let time = chrono::DateTime::parse_from_rfc3339(&now.to_string()).expect("canonical time");
    (time + chrono::Duration::days(days as i64)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
pub fn subject_key(id: &str, existing: &Value) -> String {
    let bytes = ubu_core::canonical_payload_bytes(
        &json!({"task":id,"field":"preconditions","existing_value":existing}),
    );
    format!("review:sha256:{}", super::review_hash::sha256(&bytes))
}
fn task_id(c: &AdvisoryCandidate) -> Result<&str> {
    match c.target_refs.as_slice() {
        [t] if t.object_type == ubu_core::ObjectType::Task => Ok(t.id.as_str()),
        _ => Err(internal("invalid review target")),
    }
}
struct Event {
    candidate: AdvisoryCandidate,
    state: String,
    context: Value,
    reason: Option<String>,
}
async fn history(state: &AppState, id: &str) -> Result<Vec<Event>> {
    // Event insertion order disambiguates equal or backdated timestamps. Events,
    // rather than candidate rows, count repeated defer/resurface decisions.
    let rows: Vec<(String,String,String)> = sqlx::query_as("SELECT c.payload_json,e.to_state,m.canonical_payload FROM candidate_decision_events e JOIN advisory_candidates c ON c.advisory_candidate_id=e.advisory_candidate_id JOIN mutation_envelopes m ON m.origin_device_id=json_extract(e.envelope_json,'$.origin_device_id') AND m.idempotency_key=json_extract(e.envelope_json,'$.idempotency_key') WHERE json_extract(c.payload_json,'$.target_refs[0].id')=? ORDER BY e.rowid")
        .bind(id).fetch_all(state.inner().store.pool()).await.map_err(internal)?;
    rows.into_iter()
        .filter_map(|(raw, next, payload)| {
            let c: AdvisoryCandidate = match serde_json::from_str(&raw) {
                Ok(c) => c,
                Err(e) => return Some(Err(internal(e))),
            };
            if !precondition_review::is_review(&c) {
                return None;
            }
            let p: Value = match serde_json::from_str(&payload) {
                Ok(p) => p,
                Err(e) => return Some(Err(internal(e))),
            };
            Some(Ok(Event {
                candidate: c,
                state: next,
                context: p["decision_context"].clone(),
                reason: p["input"]["rejection_reason_or_user_correction"]
                    .as_str()
                    .filter(|r| *r != "No reason provided")
                    .map(str::to_owned),
            }))
        })
        .collect()
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Interval {
    pub suggested_days: u64,
    pub seed_days: u64,
    pub ceiling_days: u64,
    pub capped: bool,
    pub evaluated_at: String,
    pub return_at: String,
    pub held_until: Option<String>,
}
pub async fn interval(state: &AppState, c: &AdvisoryCandidate) -> Result<Interval> {
    let id = task_id(c)?;
    let (seed, _) = setting_authoring::review_days(state, setting_authoring::REVIEW_SEED).await?;
    let (ceiling, _) =
        setting_authoring::review_days(state, setting_authoring::REVIEW_CEILING).await?;
    let mut count = 0u32;
    for e in history(state, id).await? {
        match e.state.as_str() {
            "admitted" => count = 0,
            "deferred" | "rejected" => count = count.saturating_add(1),
            _ => {}
        }
    }
    let escalated = seed
        .saturating_mul(1u64.checked_shl(count).unwrap_or(u64::MAX))
        .min(ceiling);
    let row = ubu_store::queries::get_current_state(state.inner().store.pool(), id).await?;
    let task: Option<Task> = row
        .map(|r| serde_json::from_str(&r.payload_json))
        .transpose()
        .map_err(internal)?;
    let universe = precondition_advisor::current(state).await?;
    let capped = task
        .and_then(|t| t.preconditions)
        .is_some_and(|tree| evaluate_universe_precondition(&universe, &tree).is_ok_and(|v| !v));
    // Blocking-now wins over escalation, even if Settings have changed. No hold
    // imposed now can cost a blocked Task more than the configured seed span.
    let suggested_days = if capped {
        escalated.min(seed)
    } else {
        escalated
    };
    let now = state.planning_now();
    let held_until = history(state, id)
        .await?
        .into_iter()
        .rev()
        .find(|e| e.candidate.advisory_candidate_id == c.advisory_candidate_id)
        .filter(|e| matches!(e.state.as_str(), "deferred" | "rejected"))
        .and_then(|e| e.context["return_at"].as_str().map(str::to_owned));
    Ok(Interval {
        suggested_days,
        seed_days: seed,
        ceiling_days: ceiling,
        capped,
        evaluated_at: now.to_string(),
        return_at: after(now, suggested_days),
        held_until,
    })
}
pub async fn decision_context(
    state: &AppState,
    c: &AdvisoryCandidate,
    requested: Option<u64>,
) -> Result<Value> {
    let policy = interval(state, c).await?;
    let days = requested.unwrap_or(policy.suggested_days);
    if days == 0 || days > policy.ceiling_days {
        return Err(AppError::bad_request_diagnostic(
            "advisory_invalid_snooze",
            "Choose a positive whole number of days no greater than the configured ceiling",
        ));
    }
    // A stale screen may offer a longer span after the facts change. Apply the
    // blocking cap at the decision boundary and report the saved date on reload.
    let actual = if policy.capped {
        days.min(policy.seed_days)
    } else {
        days
    };
    Ok(json!({"days":actual,"return_at":after(state.planning_now(),actual)}))
}

/// Called under advisory_run_lock before the model is asked. A held or active
/// subject is removed from the request, so equivalent rewordings cannot nag.
pub async fn eligible(
    state: &AppState,
    tasks: Vec<ReviewTask>,
    force: bool,
) -> Result<(Vec<ReviewTask>, Vec<Value>)> {
    let mut eligible = Vec::new();
    let mut diagnostics = Vec::new();
    for mut task in tasks {
        let key = subject_key(&task.id, &task.existing_precondition);
        let events = history(state, &task.id).await?;
        task.prior_rejection_reason = events
            .iter()
            .rev()
            .find(|e| e.state == "rejected" && e.candidate.suppression_key.as_deref() == Some(&key))
            .and_then(|e| e.reason.clone());
        let rows:Vec<String>=sqlx::query_scalar("SELECT payload_json FROM advisory_candidates WHERE suppression_key=? AND lifecycle_state IN ('proposed','resurfaced','deferred') ORDER BY rowid DESC")
            .bind(&key).fetch_all(state.inner().store.pool()).await.map_err(internal)?;
        let current = rows
            .first()
            .map(|raw| serde_json::from_str::<AdvisoryCandidate>(raw))
            .transpose()
            .map_err(internal)?;
        if current
            .as_ref()
            .is_some_and(|c| c.lifecycle_state.is_active_queue())
        {
            diagnostics.push(json!({"code":"advisory_proposal_already_queued","message":"This requirement already has a review in the queue; no duplicate was enqueued."}));
            continue;
        }
        let latest = events
            .iter()
            .rev()
            .find(|e| e.candidate.suppression_key.as_deref() == Some(&key));
        let held = latest
            .filter(|e| matches!(e.state.as_str(), "deferred" | "rejected"))
            .and_then(|e| e.context["return_at"].as_str());
        if !force
            && held.is_some_and(|date| {
                UbuTimestamp::parse(date).is_ok_and(|t| t > state.planning_now())
            })
        {
            diagnostics.push(json!({"code":"advisory_proposal_suppressed","message":format!("This review is held until {}; Review again now can reconsider it sooner.",held.unwrap())}));
            continue;
        }
        if let Some(c) = current.filter(|c| c.lifecycle_state == CandidateLifecycleState::Deferred)
        {
            let trigger = if force {
                ResurfaceTrigger::UserRequest
            } else {
                ResurfaceTrigger::PolicyReviewInterval
            };
            let envelope = state.envelope_for(
                Default::default(),
                AuthoritySource::User,
                state.planning_now(),
            )?;
            ubu_store::api::admission::transition_advisory_candidate(
                state.inner().store.pool(),
                &envelope,
                &c.advisory_candidate_id,
                c.version,
                CandidateLifecycleState::Resurfaced,
                Some(trigger),
            )
            .await?;
            diagnostics.push(json!({"code":"precondition_review_resurfaced","message":"A held review has returned to the queue for your decision."}));
            continue;
        }
        eligible.push(task);
    }
    Ok((eligible, diagnostics))
}
