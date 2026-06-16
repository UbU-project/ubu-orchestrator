use axum::Json;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::api::health::health,
        crate::api::bootstrap::start,
        crate::api::bootstrap::answer,
        crate::api::bootstrap::seed,
        crate::api::desktop::github_token,
        crate::api::github::import_fixture,
        crate::api::github::import_live,
        crate::api::planning::generate,
        crate::api::calendar::current,
        crate::api::next_action::next_action,
        crate::api::user_action::start,
        crate::api::user_action::done,
        crate::api::user_action::snooze,
        crate::api::user_action::reject,
        crate::api::user_action::decompose,
        crate::api::projection::preview,
        crate::api::projection::approve,
        crate::api::reports::risk,
        crate::api::reports::human_complete
    ),
    components(schemas(
        crate::api::health::HealthResponse,
        crate::api::bootstrap::BootstrapStartResponse,
        crate::api::bootstrap::BootstrapAnswerRequest,
        crate::api::bootstrap::BootstrapAnswerResponse,
        crate::api::bootstrap::BootstrapSeedRequest,
        crate::api::bootstrap::BootstrapSelectedRepo,
        crate::api::bootstrap::BootstrapAnswers,
        crate::api::bootstrap::WorkStyle,
        crate::api::bootstrap::AttentionPreference,
        crate::api::bootstrap::BootstrapSeedResponse,
        crate::api::bootstrap::BootstrapDiagnostic,
        crate::api::desktop::GithubTokenIntakeRequest,
        crate::api::desktop::GithubTokenIntakeResponse,
        crate::api::github::ImportFixtureRequest,
        crate::api::github::ImportLiveRequest,
        crate::api::github::ImportResponse,
        crate::api::github::ImportedCandidate,
        crate::api::planning::GeneratePlanningRequest,
        crate::api::planning::PlanningRequestBody,
        crate::api::planning::TaskSpecBody,
        crate::api::planning::PlanningResponseBody,
        crate::api::planning::PlanBody,
        crate::api::planning::ScheduledTaskBody,
        crate::api::planning::DiagnosticBody,
        crate::api::calendar::CalendarResponse,
        crate::api::next_action::NextActionResponse,
        crate::api::user_action::UserActionRequest,
        crate::api::user_action::LogEntryResponse,
        crate::api::user_action::TaskActionKind,
        crate::api::user_action::TaskLifecycleStatus,
        crate::api::projection::ProjectionPreviewRequest,
        crate::api::projection::ProjectionPreviewResponse,
        crate::api::projection::ProjectionApproveRequest,
        crate::api::projection::ProjectionResultResponse,
        crate::api::projection::AuthoritySourceBody,
        crate::api::reports::RiskReportResponse,
        crate::api::reports::HumanCompleteReportResponse,
        crate::api::reports::TaskStatusCount
    )),
    tags(
        (name = "ubu-orchestrator", description = "Local UbU Phase 1 orchestration API")
    )
)]
pub struct ApiDoc;

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
