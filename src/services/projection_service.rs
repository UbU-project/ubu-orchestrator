use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use ubu_core::core::CompartmentBoundaryDecidedPayload;
use ubu_core::id_registry::ObjectType;
use ubu_core::projection::{
    ExportGateDecision, ExportPermit, ExportProjectionContext, Legitimizer, OperationResult,
    OperationResultStatus, ProjectionApproval, ProjectionOperation, ProjectionOperationKind,
    ProjectionPreview, ProjectionResult, ProjectionResultStatus,
};
use ubu_core::{
    AuthoritySource, Legitimization, ObjectRef, PolicySummary, Provenance, SourceRef, UbuId,
    UbuTimestamp,
};
use ubu_github_adapter::auth::GitHubAuth;
use ubu_github_adapter::client::{GitHubClient, RecordingGitHubApi};
use ubu_github_adapter::errors::AdapterError;
use ubu_github_adapter::markers::is_managed_label;
use ubu_github_adapter::projection::labels::{
    apply_managed_label, managed_label_preflight, remove_managed_label,
};
use ubu_github_adapter::projection::operations::{
    GitHubProjectionOperation, GitHubProjectionOperationKind, GitHubProjectionPayload,
    GitHubProjectionTarget,
};
use ubu_github_adapter::projection::{
    apply_managed_label_write, read_managed_label_observation, GitHubLabelWrite,
};
use ubu_github_adapter::sources::GitHubRepositorySource;
use ubu_store::models::log_record::NewLogRecord;
use ubu_store::models::object_record::NewObjectRecord;
use ubu_store::models::projection_record::{NewProjectionPreviewRecord, NewProjectionResultRecord};
use ubu_store::queries;

use crate::api::projection::{
    PolicySummaryBody, ProjectionAcceptExternalRequest, ProjectionAcceptExternalResponse,
    ProjectionApproveRequest, ProjectionConflictBody, ProjectionDiagnostic,
    ProjectionOperationBody, ProjectionOperationResultBody, ProjectionPreviewRequest,
    ProjectionPreviewResponse, ProjectionReconcileRequest, ProjectionReconcileResponse,
    ProjectionResultResponse, ProjectionTargetBody, PROJECTION_APPROVAL_SCHEMA_VERSION,
    PROJECTION_EXTERNAL_ACCEPT_SCHEMA_VERSION, PROJECTION_PREVIEW_SCHEMA_VERSION,
    PROJECTION_RECONCILIATION_SCHEMA_VERSION, PROJECTION_RESULT_SCHEMA_VERSION,
};
use crate::config::ProjectionExportMode;
use crate::errors::{AppError, Result};
use crate::state::AppState;

pub async fn preview(
    state: AppState,
    request: ProjectionPreviewRequest,
) -> Result<ProjectionPreviewResponse> {
    validate_schema_version(
        request.schema_version.as_deref(),
        PROJECTION_PREVIEW_SCHEMA_VERSION,
    )?;
    validate_desired_labels(&request.desired_labels)?;

    let mut operations = desired_label_diff(&request)?;
    if operations.is_empty() {
        if let Some(operation) = managed_label_preflight(
            request.owner.clone(),
            request.repo.clone(),
            &request.existing_repository_labels,
        ) {
            operations.push(operation);
        }
    }

    let mut batch = preview_for_operations_with_existing_labels(
        operations,
        &request.existing_repository_labels,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;
    ensure_label_only_batch(&batch)?;
    let policy_summary = resolved_policy_summary(request.no_external_export);
    batch.preview.policy_summary = Some(policy_summary.clone());

    let preview_id = batch.preview.id.to_string();
    let created_at = batch.preview.created_at.to_string();
    let payload = StoredPreviewPayload {
        schema_version: PROJECTION_PREVIEW_SCHEMA_VERSION.to_owned(),
        reason: request.reason,
        preview: batch.preview.clone(),
        github_operations: batch.github_operations.clone(),
        policy_summary: policy_summary.clone(),
    };

    queries::store_projection_preview(
        state.inner().store.pool(),
        NewProjectionPreviewRecord {
            id: preview_id.clone(),
            request_id: preview_id.clone(),
            status: "pending".to_owned(),
            payload: serde_json::to_value(&payload)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            created_at,
        },
    )
    .await
    .map_err(AppError::from)?;

    Ok(ProjectionPreviewResponse {
        schema_version: PROJECTION_PREVIEW_SCHEMA_VERSION.to_owned(),
        preview_id,
        operations: batch
            .github_operations
            .iter()
            .map(operation_body)
            .collect::<Result<Vec<_>>>()?,
        policy_summary: policy_summary_body(&policy_summary)?,
        requires_approval: true,
    })
}

pub async fn approve(
    state: AppState,
    request: ProjectionApproveRequest,
) -> Result<ProjectionResultResponse> {
    validate_schema_version(
        request.schema_version.as_deref(),
        PROJECTION_APPROVAL_SCHEMA_VERSION,
    )?;
    let preview_id = UbuId::parse(request.preview_id.clone())
        .map_err(|e| AppError::BadRequest(format!("invalid preview id: {e}")))?;
    let pool = state.inner().store.pool();
    let stored = load_preview(pool, preview_id.as_str()).await?;
    let approved_at = match request.approved_at {
        Some(value) => UbuTimestamp::parse(value)
            .map_err(|e| AppError::BadRequest(format!("invalid approved_at: {e}")))?,
        None => UbuTimestamp::now_utc(),
    };
    let requested_authority: AuthoritySource = request.authority_source.into();

    let approval = ProjectionApproval {
        preview_id,
        approved: request.approved,
        approved_at: approved_at.clone(),
        authority_source: requested_authority,
    };
    persist_approval(pool, &approval).await?;

    if !approval.approved {
        let operation_results = stored
            .github_operations
            .iter()
            .map(|operation| OperationResult {
                operation_id: operation.operation_id.clone(),
                status: OperationResultStatus::Skipped,
                message: Some("projection batch was not approved".to_owned()),
            })
            .collect::<Vec<_>>();
        let result = ProjectionResult {
            preview_id: stored.preview.id.clone(),
            applied_at: UbuTimestamp::now_utc(),
            status: ProjectionResultStatus::Failed,
            operation_results,
        };
        persist_result(pool, &result, Vec::new()).await?;
        return result_response(&result, Vec::new());
    }

    let stored_batch = batch_from_stored(&stored);
    ensure_label_only_batch(&stored_batch)?;
    let policy_summary = stored
        .preview
        .policy_summary
        .clone()
        .unwrap_or_else(|| stored.policy_summary.clone());
    let Some(github_client) = projection_github_client(&state).await? else {
        let operation_results = stored
            .github_operations
            .iter()
            .map(|operation| OperationResult {
                operation_id: operation.operation_id.clone(),
                status: OperationResultStatus::Failed,
                message: Some(
                    "live GitHub projection export has no desktop session token".to_owned(),
                ),
            })
            .collect::<Vec<_>>();
        let diagnostics = vec![ProjectionDiagnostic {
            code: "missing_github_session_token".to_owned(),
            message: "live GitHub projection export requires an in-memory desktop session token"
                .to_owned(),
            operation_id: None,
        }];
        let result = ProjectionResult {
            preview_id: stored.preview.id.clone(),
            applied_at: UbuTimestamp::now_utc(),
            status: ProjectionResultStatus::Failed,
            operation_results,
        };
        persist_result(pool, &result, diagnostics.clone()).await?;
        return result_response(&result, diagnostics);
    };

    let mut operation_results = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, operation) in stored.github_operations.iter().enumerate() {
        let core_operation = preview_operation(&stored.preview, operation)?;
        let adjudication = gate_export_operation(
            core_operation,
            operation,
            Some(&policy_summary),
            AuthoritySource::AutomationWorker,
        );
        append_boundary_log(
            pool,
            &stored.preview,
            operation,
            &adjudication.decision.log_payload,
        )
        .await?;

        match adjudication.decision.legitimization {
            Legitimization::Accepted => {
                let Some(permit) = adjudication.permit() else {
                    operation_results.push(OperationResult {
                        operation_id: operation.operation_id.clone(),
                        status: OperationResultStatus::Failed,
                        message: Some(
                            "accepted projection export did not include a core export permit"
                                .to_owned(),
                        ),
                    });
                    continue;
                };
                if let Err(message) = assert_operation_payload_managed(operation) {
                    operation_results.push(OperationResult {
                        operation_id: operation.operation_id.clone(),
                        status: OperationResultStatus::Failed,
                        message: Some(message),
                    });
                    continue;
                }
                if let Err(error) = ensure_permit_matches(operation, permit) {
                    operation_results.push(OperationResult {
                        operation_id: operation.operation_id.clone(),
                        status: OperationResultStatus::Failed,
                        message: Some(error.to_string()),
                    });
                    continue;
                }
                match apply_managed_label_operation(&github_client, operation, permit).await {
                    Ok(message) => operation_results.push(OperationResult {
                        operation_id: operation.operation_id.clone(),
                        status: OperationResultStatus::Applied,
                        message: Some(message),
                    }),
                    Err(error) => {
                        let message = error.to_string();
                        operation_results.push(OperationResult {
                            operation_id: operation.operation_id.clone(),
                            status: OperationResultStatus::Failed,
                            message: Some(message.clone()),
                        });
                        if error.is_rate_limit_or_transport_failure() {
                            diagnostics.push(ProjectionDiagnostic {
                                code: "github_projection_transport_aborted".to_owned(),
                                message:
                                    "GitHub projection batch stopped after a rate-limit or transport failure"
                                        .to_owned(),
                                operation_id: Some(operation.operation_id.clone()),
                            });
                            operation_results.extend(
                                stored.github_operations[index + 1..]
                                    .iter()
                                    .map(|remaining| OperationResult {
                                        operation_id: remaining.operation_id.clone(),
                                        status: OperationResultStatus::Skipped,
                                        message: Some(
                                            "not applied because an earlier GitHub write hit a rate-limit or transport failure"
                                                .to_owned(),
                                        ),
                                    }),
                            );
                            break;
                        }
                    }
                }
            }
            Legitimization::NeedsReview | Legitimization::Rejected => {
                let reason = denial_reason(&policy_summary);
                diagnostics.push(ProjectionDiagnostic {
                    code: "projection_denied".to_owned(),
                    message: reason.clone(),
                    operation_id: Some(operation.operation_id.clone()),
                });
                operation_results.push(OperationResult {
                    operation_id: operation.operation_id.clone(),
                    status: OperationResultStatus::Skipped,
                    message: Some(reason),
                });
            }
        }
    }

    let result = ProjectionResult {
        preview_id: stored.preview.id.clone(),
        applied_at: UbuTimestamp::now_utc(),
        status: result_status(&operation_results),
        operation_results,
    };
    persist_result(pool, &result, diagnostics.clone()).await?;
    result_response(&result, diagnostics)
}

pub async fn reconcile(
    state: AppState,
    request: ProjectionReconcileRequest,
) -> Result<ProjectionReconcileResponse> {
    validate_schema_version(
        request.schema_version.as_deref(),
        PROJECTION_RECONCILIATION_SCHEMA_VERSION,
    )?;

    let pool = state.inner().store.pool();
    let last_result = load_last_applied_result(pool).await?;
    let preview = load_preview(pool, &last_result.preview_id).await?;
    let preview_batch = batch_from_stored(&preview);
    let observed = match state.inner().config.github_projection_export_mode() {
        ProjectionExportMode::Live => {
            let Some(github_client) = projection_github_client(&state).await? else {
                return Err(AppError::bad_request_diagnostic(
                    "missing_github_session_token",
                    "live GitHub projection reconciliation requires an in-memory desktop session token",
                ));
            };
            read_live_managed_labels(&github_client, &preview_batch).await?
        }
        ProjectionExportMode::Mock => request.observed_labels.into_iter().collect::<BTreeSet<_>>(),
    };
    let conflicts = reconciliation_conflicts(&preview_batch, &last_result.result, &observed);
    let status = if conflicts.is_empty() {
        "matched"
    } else if conflicts
        .iter()
        .any(|conflict| conflict.conflict_type == "drifted")
    {
        "drifted"
    } else {
        "missing"
    };
    let diagnostics = if conflicts.is_empty() {
        Vec::new()
    } else {
        vec![ProjectionDiagnostic {
            code: "projection_conflict".to_owned(),
            message: "observed GitHub labels differ from the last applied projection".to_owned(),
            operation_id: None,
        }]
    };
    let reconciliation_id = UbuId::new(ObjectType::Snapshot).to_string();
    let now = UbuTimestamp::now_utc().to_string();
    let payload = json!({
        "schema_version": PROJECTION_RECONCILIATION_SCHEMA_VERSION,
        "preview_id": last_result.preview_id,
        "result_id": last_result.result_id,
        "status": status,
        "observed_labels": observed.iter().cloned().collect::<Vec<_>>(),
        "conflicts": conflicts,
        "diagnostics": diagnostics,
    });

    sqlx::query(
        "INSERT INTO projection_reconciliations
        (id, preview_id, result_id, status, payload_json, created_at)
        VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&reconciliation_id)
    .bind(&last_result.preview_id)
    .bind(&last_result.result_id)
    .bind(status)
    .bind(serde_json::to_string(&payload).map_err(|e| AppError::Internal(e.to_string()))?)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(ProjectionReconcileResponse {
        schema_version: PROJECTION_RECONCILIATION_SCHEMA_VERSION.to_owned(),
        reconciliation_id,
        preview_id: last_result.preview_id,
        status: status.to_owned(),
        conflicts,
        diagnostics,
    })
}

pub async fn accept_external(
    state: AppState,
    request: ProjectionAcceptExternalRequest,
) -> Result<ProjectionAcceptExternalResponse> {
    validate_schema_version(
        request.schema_version.as_deref(),
        PROJECTION_EXTERNAL_ACCEPT_SCHEMA_VERSION,
    )?;
    let pool = state.inner().store.pool();
    let row = sqlx::query("SELECT payload_json FROM projection_reconciliations WHERE id = ?")
        .bind(&request.reconciliation_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("projection reconciliation not found".to_owned()))?;
    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let payload: ReconciliationPayload =
        serde_json::from_str(&payload_json).map_err(|e| AppError::Internal(e.to_string()))?;
    let conflict = payload
        .conflicts
        .iter()
        .find(|conflict| conflict.operation_id == request.conflict_operation_id)
        .ok_or_else(|| {
            AppError::bad_request_diagnostic(
                "unknown_projection_conflict",
                "conflict_operation_id is not present in the reconciliation",
            )
        })?;

    let now = UbuTimestamp::now_utc().to_string();
    let admitted_object_id = UbuId::new(ObjectType::ExternalEvent).to_string();
    let authority_source: AuthoritySource = request.authority_source.into();
    let authority_source_wire = authority_source_wire(authority_source)?;
    let source = SourceRef {
        source_kind: "github".to_owned(),
        source_id: format!("{}:{}", request.reconciliation_id, conflict.operation_id),
        url: None,
    };
    let object_payload = json!({
        "id": admitted_object_id,
        "source": source,
        "event_type": "github_projection_external_change_accepted",
        "occurred_at": now,
        "payload": {
            "schema_version": PROJECTION_EXTERNAL_ACCEPT_SCHEMA_VERSION,
            "reconciliation_id": request.reconciliation_id,
            "conflict": conflict,
            "provenance": {
                "created_at": now,
                "authority_source": authority_source_wire,
                "source": {
                    "source_kind": "github",
                    "source_id": format!("{}:{}", payload.preview_id, conflict.operation_id),
                    "url": Value::Null
                }
            }
        }
    });

    queries::admit_object(
        pool,
        NewObjectRecord {
            id: admitted_object_id.clone(),
            object_type: ObjectType::ExternalEvent.as_str().to_owned(),
            version: 1,
            status: "active".to_owned(),
            compartment_label: "github-import".to_owned(),
            payload: object_payload,
            created_at: now.clone(),
            updated_at: now,
        },
    )
    .await
    .map_err(AppError::from)?;

    Ok(ProjectionAcceptExternalResponse {
        schema_version: PROJECTION_EXTERNAL_ACCEPT_SCHEMA_VERSION.to_owned(),
        admitted_object_id,
        reconciliation_id: request.reconciliation_id,
        conflict_operation_id: request.conflict_operation_id,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredPreviewPayload {
    schema_version: String,
    reason: Option<String>,
    preview: ProjectionPreview,
    github_operations: Vec<GitHubProjectionOperation>,
    policy_summary: PolicySummary,
}

#[derive(Debug, Clone)]
struct ProjectionPreviewBatch {
    preview: ProjectionPreview,
    github_operations: Vec<GitHubProjectionOperation>,
}

#[derive(Debug, Clone)]
struct StoredProjectionResult {
    result_id: String,
    preview_id: String,
    result: ProjectionResult,
}

#[derive(Debug, Clone, Deserialize)]
struct ReconciliationPayload {
    preview_id: String,
    conflicts: Vec<ProjectionConflictBody>,
}

fn validate_schema_version(actual: Option<&str>, expected: &str) -> Result<()> {
    match actual {
        Some(value) if value == expected => Ok(()),
        Some(other) => Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!("unsupported schema_version `{other}`"),
        )),
        None => Err(AppError::bad_request_diagnostic(
            "missing_schema_version",
            "schema_version is required",
        )),
    }
}

fn validate_desired_labels(labels: &[String]) -> Result<()> {
    for label in labels {
        if !is_managed_label(label) {
            return Err(AppError::bad_request_diagnostic(
                "unmanaged_projection_label",
                format!("projection labels are limited to UbU-managed labels, got `{label}`"),
            ));
        }
    }
    Ok(())
}

fn desired_label_diff(
    request: &ProjectionPreviewRequest,
) -> Result<Vec<GitHubProjectionOperation>> {
    let Some(issue_number) = request.issue_number else {
        return Ok(Vec::new());
    };

    let desired = request
        .desired_labels
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let observed = request
        .observed_labels
        .iter()
        .filter(|label| is_managed_label(label))
        .cloned()
        .collect::<BTreeSet<_>>();
    let target =
        GitHubProjectionTarget::issue(request.owner.clone(), request.repo.clone(), issue_number);

    let mut operations = Vec::new();
    for label in desired.difference(&observed) {
        operations.push(apply_managed_label(
            stable_operation_id("apply", &request.owner, &request.repo, issue_number, label),
            target.clone(),
            label.clone(),
        ));
    }
    for label in observed.difference(&desired) {
        operations.push(remove_managed_label(
            stable_operation_id("remove", &request.owner, &request.repo, issue_number, label),
            target.clone(),
            label.clone(),
        ));
    }
    Ok(operations)
}

fn stable_operation_id(
    action: &str,
    owner: &str,
    repo: &str,
    issue_number: u64,
    label: &str,
) -> String {
    format!(
        "label-{action}-{}-{}-{issue_number}-{}",
        owner.to_ascii_lowercase().replace('/', "-"),
        repo.to_ascii_lowercase().replace('/', "-"),
        label.to_ascii_lowercase().replace('/', "-")
    )
}

fn resolved_policy_summary(no_external_export: bool) -> PolicySummary {
    let checked_at = UbuTimestamp::now_utc();
    if no_external_export {
        return PolicySummary {
            legitimization: Legitimization::Rejected,
            adjudication_reasons: vec![
                "effective compartment policy forbids external export".to_owned()
            ],
            local_only: Some(false),
            no_cloud_llm: Some(false),
            no_external_export: Some(true),
            checked_at,
        };
    }

    PolicySummary {
        legitimization: Legitimization::Accepted,
        adjudication_reasons: vec![
            "managed-label projection is allowed for automation worker export".to_owned(),
        ],
        local_only: Some(false),
        no_cloud_llm: Some(false),
        no_external_export: Some(false),
        checked_at,
    }
}

async fn load_preview(pool: &sqlx::SqlitePool, preview_id: &str) -> Result<StoredPreviewPayload> {
    let row = sqlx::query("SELECT payload_json FROM projection_previews WHERE id = ?")
        .bind(preview_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound("projection preview not found".to_owned()))?;
    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let payload: StoredPreviewPayload =
        serde_json::from_str(&payload_json).map_err(|e| AppError::Internal(e.to_string()))?;
    if payload.schema_version != PROJECTION_PREVIEW_SCHEMA_VERSION {
        return Err(AppError::bad_request_diagnostic(
            "unknown_schema_version",
            format!(
                "stored preview has unsupported schema_version `{}`",
                payload.schema_version
            ),
        ));
    }
    Ok(payload)
}

fn batch_from_stored(stored: &StoredPreviewPayload) -> ProjectionPreviewBatch {
    ProjectionPreviewBatch {
        preview: stored.preview.clone(),
        github_operations: stored.github_operations.clone(),
    }
}

fn preview_for_operations_with_existing_labels(
    operations: Vec<GitHubProjectionOperation>,
    existing_labels: &[String],
) -> Result<ProjectionPreviewBatch> {
    let operations = with_managed_label_preflight(operations, existing_labels);
    let core_operations = operations
        .iter()
        .map(core_operation_from_github)
        .collect::<Result<Vec<_>>>()?;
    Ok(ProjectionPreviewBatch {
        preview: ProjectionPreview {
            id: UbuId::new(ObjectType::ProjectionPreview),
            created_at: UbuTimestamp::now_utc(),
            operations: core_operations,
            policy_summary: None,
        },
        github_operations: operations,
    })
}

fn with_managed_label_preflight(
    operations: Vec<GitHubProjectionOperation>,
    existing_labels: &[String],
) -> Vec<GitHubProjectionOperation> {
    let mut repositories = operations
        .iter()
        .map(|operation| {
            (
                operation.target.owner.as_str(),
                operation.target.repo.as_str(),
            )
        })
        .collect::<Vec<_>>();
    repositories.sort_unstable();
    repositories.dedup();

    let mut preflight = repositories
        .into_iter()
        .filter_map(|(owner, repo)| managed_label_preflight(owner, repo, existing_labels))
        .collect::<Vec<_>>();
    preflight.extend(operations);
    preflight
}

fn core_operation_from_github(
    operation: &GitHubProjectionOperation,
) -> Result<ProjectionOperation> {
    let kind = match operation.kind {
        GitHubProjectionOperationKind::ManagedLabelPreflight
        | GitHubProjectionOperationKind::ApplyLabel
        | GitHubProjectionOperationKind::RemoveLabel => ProjectionOperationKind::Label,
        GitHubProjectionOperationKind::CreateComment => ProjectionOperationKind::Comment,
        GitHubProjectionOperationKind::CreateManagedIssue => ProjectionOperationKind::Create,
    };
    Ok(ProjectionOperation {
        operation_id: operation.operation_id.clone(),
        kind,
        target: github_target_source_ref(&operation.target, "github"),
        summary: operation.summary.clone(),
        payload: Some(
            serde_json::to_value(operation).map_err(|e| AppError::Internal(e.to_string()))?,
        ),
    })
}

fn github_target_source_ref(target: &GitHubProjectionTarget, kind: &str) -> SourceRef {
    let source_id = match target.issue_number {
        Some(number) => format!("{}/{}#{}", target.owner, target.repo, number),
        None => format!("{}/{}", target.owner, target.repo),
    };
    SourceRef {
        source_kind: kind.to_owned(),
        source_id,
        url: Some(format!(
            "https://github.com/{}/{}",
            target.owner, target.repo
        )),
    }
}

fn ensure_label_only_batch(batch: &ProjectionPreviewBatch) -> Result<()> {
    for operation in &batch.github_operations {
        let is_label_operation = matches!(
            (&operation.kind, &operation.payload),
            (
                GitHubProjectionOperationKind::ManagedLabelPreflight,
                GitHubProjectionPayload::ManagedLabelPreflight(_)
            ) | (
                GitHubProjectionOperationKind::ApplyLabel
                    | GitHubProjectionOperationKind::RemoveLabel,
                GitHubProjectionPayload::Label { .. }
            )
        );
        if !is_label_operation {
            return Err(AppError::bad_request_diagnostic(
                "unsupported_projection_operation",
                "O7 projection writes are limited to managed-label operations",
            ));
        }
    }
    for operation in &batch.preview.operations {
        if !matches!(operation.kind, ProjectionOperationKind::Label) {
            return Err(AppError::bad_request_diagnostic(
                "unsupported_projection_operation",
                "O7 projection previews may contain only label operations",
            ));
        }
    }
    Ok(())
}

async fn persist_approval(pool: &sqlx::SqlitePool, approval: &ProjectionApproval) -> Result<()> {
    let id = UbuId::new(ObjectType::Snapshot).to_string();
    let created_at = UbuTimestamp::now_utc().to_string();
    let authority_source = authority_source_wire(approval.authority_source)?;
    let payload = json!({
        "schema_version": PROJECTION_APPROVAL_SCHEMA_VERSION,
        "preview_id": approval.preview_id,
        "approved": approval.approved,
        "authority_source": authority_source,
        "approved_at": approval.approved_at,
    });
    sqlx::query(
        "INSERT INTO projection_approvals
        (id, preview_id, approved, authority_source, payload_json, approved_at, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(approval.preview_id.to_string())
    .bind(if approval.approved { 1_i64 } else { 0_i64 })
    .bind(authority_source)
    .bind(serde_json::to_string(&payload).map_err(|e| AppError::Internal(e.to_string()))?)
    .bind(approval.approved_at.to_string())
    .bind(created_at)
    .execute(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
}

fn preview_operation<'a>(
    preview: &'a ProjectionPreview,
    operation: &GitHubProjectionOperation,
) -> Result<&'a ProjectionOperation> {
    preview
        .operations
        .iter()
        .find(|candidate| candidate.operation_id == operation.operation_id)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "stored preview is missing core projection operation `{}`",
                operation.operation_id
            ))
        })
}

fn gate_export_operation(
    core_operation: &ProjectionOperation,
    github_operation: &GitHubProjectionOperation,
    effective_policy: Option<&PolicySummary>,
    authority_source: AuthoritySource,
) -> ExportGateDecision {
    let effective_time = UbuTimestamp::now_utc();
    let compartment_ref = ObjectRef {
        id: UbuId::new(ObjectType::Compartment),
        object_type: ObjectType::Compartment,
    };
    let actor_identity_ref = ObjectRef {
        id: UbuId::new(ObjectType::Identity),
        object_type: ObjectType::Identity,
    };
    let provenance = Provenance {
        created_at: effective_time.clone(),
        created_by: None,
        authority_source,
        source: Some(github_target_source_ref(&github_operation.target, "github")),
        source_refs: None,
    };

    Legitimizer::gate_export_projection(ExportProjectionContext {
        operation: core_operation,
        effective_policy,
        compartment_ref: &compartment_ref,
        actor_identity_ref: &actor_identity_ref,
        authority_source,
        effective_time,
        provenance: &provenance,
    })
}

async fn append_boundary_log(
    pool: &sqlx::SqlitePool,
    preview: &ProjectionPreview,
    operation: &GitHubProjectionOperation,
    log_payload: &CompartmentBoundaryDecidedPayload,
) -> Result<()> {
    queries::append_log_entry(
        pool,
        NewLogRecord {
            id: UbuId::new(ObjectType::LogEntry).to_string(),
            event_type: "compartment_boundary_decided".to_owned(),
            object_refs: json!([preview.id.to_string(), operation.operation_id]),
            payload: serde_json::to_value(log_payload)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            provenance: serde_json::to_value(&log_payload.provenance)
                .map_err(|e| AppError::Internal(e.to_string()))?,
            created_at: log_payload.effective_time.to_string(),
        },
    )
    .await
    .map_err(AppError::from)?;
    Ok(())
}

async fn projection_github_client(state: &AppState) -> Result<Option<GitHubClient>> {
    match state.inner().config.github_projection_export_mode() {
        ProjectionExportMode::Mock => Ok(Some(GitHubClient::recording(Arc::new(
            RecordingGitHubApi::new(),
        )))),
        ProjectionExportMode::Live => {
            let token = state.inner().desktop_session_token.lock().await.clone();
            let Some(token) = token else {
                return Ok(None);
            };
            let auth = GitHubAuth::from_session_token(token.expose_for_adapter().to_owned())
                .map_err(adapter_app_error)?;
            GitHubClient::from_auth(auth)
                .map(Some)
                .map_err(adapter_app_error)
        }
    }
}

async fn apply_managed_label_operation(
    client: &GitHubClient,
    operation: &GitHubProjectionOperation,
    permit: &ExportPermit,
) -> std::result::Result<String, AdapterError> {
    match (&operation.kind, &operation.payload) {
        (
            GitHubProjectionOperationKind::ManagedLabelPreflight,
            GitHubProjectionPayload::ManagedLabelPreflight(payload),
        ) => {
            for label in &payload.missing_labels {
                let color = if label == "ubu" { "5319e7" } else { "0e8a16" };
                client
                    .api()
                    .create_label(
                        &operation.target.owner,
                        &operation.target.repo,
                        label,
                        color,
                        "UbU managed label",
                    )
                    .await?;
            }
            Ok("managed labels ensured".to_owned())
        }
        (GitHubProjectionOperationKind::ApplyLabel, GitHubProjectionPayload::Label { label }) => {
            let issue_number = issue_number(operation)?;
            let payload = GitHubLabelWrite {
                repository: repository_source(&operation.target),
                issue_number,
                labels: vec![label.clone()],
            };
            let result =
                apply_managed_label_write(client, &payload, permit.authority_source()).await?;
            Ok(format!(
                "managed labels applied: {}",
                result.applied_labels.join(", ")
            ))
        }
        (GitHubProjectionOperationKind::RemoveLabel, GitHubProjectionPayload::Label { label }) => {
            let issue_number = issue_number(operation)?;
            client
                .api()
                .remove_label_from_issue(
                    &operation.target.owner,
                    &operation.target.repo,
                    issue_number,
                    label,
                )
                .await?;
            Ok(format!("managed label removed: {label}"))
        }
        _ => Err(AdapterError::ForbiddenProjectionOperation {
            reason: format!(
                "operation {} has mismatched payload",
                operation.operation_id
            ),
        }),
    }
}

async fn read_live_managed_labels(
    client: &GitHubClient,
    preview: &ProjectionPreviewBatch,
) -> Result<BTreeSet<String>> {
    let Some(target) = preview
        .github_operations
        .iter()
        .map(|operation| &operation.target)
        .find(|target| target.issue_number.is_some())
    else {
        return Err(AppError::bad_request_diagnostic(
            "missing_projection_issue_target",
            "live GitHub projection reconciliation requires an issue target",
        ));
    };

    let issue_number = target.issue_number.ok_or_else(|| {
        AppError::bad_request_diagnostic(
            "missing_projection_issue_target",
            "live GitHub projection reconciliation requires an issue target",
        )
    })?;
    let observation =
        read_managed_label_observation(client, &repository_source(target), issue_number)
            .await
            .map_err(adapter_app_error)?;
    Ok(observation.labels.into_iter().collect())
}

fn ensure_permit_matches(
    operation: &GitHubProjectionOperation,
    permit: &ExportPermit,
) -> Result<()> {
    if permit.operation_id() != operation.operation_id {
        return Err(AppError::Internal(format!(
            "core export permit for `{}` cannot authorize operation `{}`",
            permit.operation_id(),
            operation.operation_id
        )));
    }
    Ok(())
}

fn assert_operation_payload_managed(
    operation: &GitHubProjectionOperation,
) -> std::result::Result<(), String> {
    match &operation.payload {
        GitHubProjectionPayload::ManagedLabelPreflight(payload) => {
            for label in &payload.missing_labels {
                if !is_managed_label(label) {
                    return Err(format!(
                        "managed-label preflight contains unmanaged label `{label}`"
                    ));
                }
            }
        }
        GitHubProjectionPayload::Label { label } if is_managed_label(label) => {}
        _ => {
            return Err("only managed-label writes are allowed in O19".to_owned());
        }
    }
    Ok(())
}

fn issue_number(operation: &GitHubProjectionOperation) -> std::result::Result<u64, AdapterError> {
    operation
        .target
        .issue_number
        .ok_or_else(|| AdapterError::UnsupportedProjectionTarget {
            source_kind: "github_repository".to_owned(),
        })
}

fn repository_source(target: &GitHubProjectionTarget) -> GitHubRepositorySource {
    GitHubRepositorySource {
        owner: target.owner.clone(),
        name: target.repo.clone(),
        default_branch: "main".to_owned(),
        html_url: format!("https://github.com/{}/{}", target.owner, target.repo),
        api_id: 0,
    }
}

fn adapter_app_error(error: AdapterError) -> AppError {
    if error.is_rate_limit_or_transport_failure() {
        AppError::Upstream(error.to_string())
    } else {
        AppError::Internal(error.to_string())
    }
}

fn denial_reason(policy_summary: &PolicySummary) -> String {
    if policy_summary.adjudication_reasons.is_empty() {
        return "projection operation was not accepted by the enforcement gate".to_owned();
    }
    policy_summary.adjudication_reasons.join("; ")
}

fn result_status(operation_results: &[OperationResult]) -> ProjectionResultStatus {
    if operation_results
        .iter()
        .all(|result| result.status == OperationResultStatus::Applied)
    {
        ProjectionResultStatus::Applied
    } else if operation_results
        .iter()
        .any(|result| result.status == OperationResultStatus::Applied)
    {
        ProjectionResultStatus::Partial
    } else {
        ProjectionResultStatus::Failed
    }
}

async fn persist_result(
    pool: &sqlx::SqlitePool,
    result: &ProjectionResult,
    diagnostics: Vec<ProjectionDiagnostic>,
) -> Result<String> {
    let result_id = UbuId::new(ObjectType::Snapshot).to_string();
    let status = projection_result_status_wire(result.status)?;
    let payload = json!({
        "schema_version": PROJECTION_RESULT_SCHEMA_VERSION,
        "result": result,
        "diagnostics": diagnostics,
    });
    queries::store_projection_result(
        pool,
        NewProjectionResultRecord {
            id: result_id.clone(),
            preview_id: result.preview_id.to_string(),
            status,
            payload,
            created_at: result.applied_at.to_string(),
        },
    )
    .await
    .map_err(AppError::from)?;
    Ok(result_id)
}

fn result_response(
    result: &ProjectionResult,
    diagnostics: Vec<ProjectionDiagnostic>,
) -> Result<ProjectionResultResponse> {
    Ok(ProjectionResultResponse {
        schema_version: PROJECTION_RESULT_SCHEMA_VERSION.to_owned(),
        preview_id: result.preview_id.to_string(),
        status: projection_result_status_wire(result.status)?,
        operation_results: result
            .operation_results
            .iter()
            .map(|result| {
                Ok(ProjectionOperationResultBody {
                    operation_id: result.operation_id.clone(),
                    status: operation_result_status_wire(result.status)?,
                    message: result.message.clone(),
                    authority_source: if result.status == OperationResultStatus::Applied {
                        Some(authority_source_wire(AuthoritySource::AutomationWorker)?)
                    } else {
                        None
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?,
        diagnostics,
    })
}

async fn load_last_applied_result(pool: &sqlx::SqlitePool) -> Result<StoredProjectionResult> {
    let row = sqlx::query(
        "SELECT id, preview_id, payload_json FROM projection_results
        WHERE status IN ('applied', 'partial')
        ORDER BY created_at DESC
        LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?
    .ok_or_else(|| AppError::NotFound("no applied projection result found".to_owned()))?;
    let result_id: String = row
        .try_get("id")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let preview_id: String = row
        .try_get("preview_id")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let payload_json: String = row
        .try_get("payload_json")
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let payload: Value =
        serde_json::from_str(&payload_json).map_err(|e| AppError::Internal(e.to_string()))?;
    let result: ProjectionResult = serde_json::from_value(payload["result"].clone())
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(StoredProjectionResult {
        result_id,
        preview_id,
        result,
    })
}

fn reconciliation_conflicts(
    preview: &ProjectionPreviewBatch,
    result: &ProjectionResult,
    observed: &BTreeSet<String>,
) -> Vec<ProjectionConflictBody> {
    let applied = result
        .operation_results
        .iter()
        .filter(|result| result.status == OperationResultStatus::Applied)
        .map(|result| result.operation_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut conflicts = Vec::new();

    for operation in &preview.github_operations {
        if !applied.contains(operation.operation_id.as_str()) {
            continue;
        }
        match &operation.payload {
            GitHubProjectionPayload::ManagedLabelPreflight(payload) => {
                for label in &payload.missing_labels {
                    if !observed.contains(label) {
                        conflicts.push(conflict(
                            operation,
                            "missing",
                            label,
                            observed,
                            "managed repository label is missing from observed GitHub state",
                        ));
                    }
                }
            }
            GitHubProjectionPayload::Label { label }
                if operation.kind == GitHubProjectionOperationKind::ApplyLabel =>
            {
                if !observed.contains(label) {
                    conflicts.push(conflict(
                        operation,
                        "missing",
                        label,
                        observed,
                        "applied managed label is missing from observed GitHub state",
                    ));
                }
            }
            GitHubProjectionPayload::Label { label }
                if operation.kind == GitHubProjectionOperationKind::RemoveLabel =>
            {
                if observed.contains(label) {
                    conflicts.push(conflict(
                        operation,
                        "drifted",
                        label,
                        observed,
                        "removed managed label is still present in observed GitHub state",
                    ));
                }
            }
            _ => {}
        }
    }

    conflicts
}

fn conflict(
    operation: &GitHubProjectionOperation,
    conflict_type: &str,
    label: &str,
    observed: &BTreeSet<String>,
    message: &str,
) -> ProjectionConflictBody {
    ProjectionConflictBody {
        operation_id: operation.operation_id.clone(),
        conflict_type: conflict_type.to_owned(),
        expected_label: label.to_owned(),
        observed_labels: observed.iter().cloned().collect(),
        message: message.to_owned(),
    }
}

fn operation_body(operation: &GitHubProjectionOperation) -> Result<ProjectionOperationBody> {
    Ok(ProjectionOperationBody {
        operation_id: operation.operation_id.clone(),
        kind: "label".to_owned(),
        target: ProjectionTargetBody {
            owner: operation.target.owner.clone(),
            repo: operation.target.repo.clone(),
            issue_number: operation.target.issue_number,
        },
        summary: operation.summary.clone(),
        payload: serde_json::to_value(&operation.payload)
            .map_err(|e| AppError::Internal(e.to_string()))?,
    })
}

fn policy_summary_body(policy_summary: &PolicySummary) -> Result<PolicySummaryBody> {
    Ok(PolicySummaryBody {
        legitimization: legitimization_wire(policy_summary.legitimization)?,
        adjudication_reasons: policy_summary.adjudication_reasons.clone(),
        local_only: policy_summary.local_only,
        no_cloud_llm: policy_summary.no_cloud_llm,
        no_external_export: policy_summary.no_external_export,
        checked_at: policy_summary.checked_at.to_string(),
    })
}

fn projection_result_status_wire(status: ProjectionResultStatus) -> Result<String> {
    wire_string(status)
}

fn operation_result_status_wire(status: OperationResultStatus) -> Result<String> {
    wire_string(status)
}

fn legitimization_wire(legitimization: Legitimization) -> Result<String> {
    wire_string(legitimization)
}

fn authority_source_wire(authority_source: AuthoritySource) -> Result<String> {
    wire_string(authority_source)
}

fn wire_string<T: Serialize>(value: T) -> Result<String> {
    let serialized =
        serde_json::to_string(&value).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(serialized.trim_matches('"').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_authority_export_context_is_rejected_without_permit() {
        let github_operation = apply_managed_label(
            "label-apply-ubu-project-ubu-orchestrator-7-ubu-managed",
            GitHubProjectionTarget::issue("UbU-project", "ubu-orchestrator", 7),
            "ubu-managed",
        );
        let batch = preview_for_operations_with_existing_labels(
            vec![github_operation.clone()],
            &["ubu".to_owned(), "ubu-managed".to_owned()],
        )
        .expect("preview batch");
        let policy_summary = resolved_policy_summary(false);
        let core_operation =
            preview_operation(&batch.preview, &github_operation).expect("core operation");

        let adjudication = gate_export_operation(
            core_operation,
            &github_operation,
            Some(&policy_summary),
            AuthoritySource::User,
        );

        assert_eq!(
            adjudication.decision.legitimization,
            Legitimization::Rejected
        );
        assert!(adjudication.permit().is_none());
        assert_eq!(
            adjudication.decision.log_payload.adjudication_result,
            Legitimization::Rejected
        );
        assert_eq!(
            adjudication.decision.log_payload.authority_source,
            AuthoritySource::User
        );
        assert!(adjudication
            .decision
            .adjudication_reasons
            .iter()
            .any(|reason| reason.contains("user-equivalent")));
    }
}
