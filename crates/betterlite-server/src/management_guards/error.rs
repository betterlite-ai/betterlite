use axum::http::StatusCode;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManagementGuardError {
    #[error("spend must be a finite number. Received: {received}")]
    NonFiniteSpend { received: String },
    #[error("{message}")]
    BudgetDuration { message: String, status: StatusCode },
    #[error("model_max_budget keys must be non-empty model names")]
    EmptyModelName,
    #[error(
        "model_max_budget['{model}'].max_budget must be a non-negative finite number. Received: {received}"
    )]
    ModelMaxBudget { model: String, received: String },
    #[error("model_max_budget['{model}'] requires a budget_duration, e.g. '1d' or '30d'")]
    ModelBudgetDuration { model: String },
    #[error(
        "model_max_budget['{model}'] tpm_limit/rpm_limit are not enforced on a team; set per-model rate limits on the key instead"
    )]
    ModelRateLimit { model: String },
    #[error(
        "Service-account keys cannot query user analytics. Use a user-bound key, or call as a proxy admin."
    )]
    ServiceAccountAnalytics,
    #[error("Only proxy admins can set `allowed_passthrough_routes` on a {entity}.")]
    PassthroughRoutes { entity: String },
    #[error("Only proxy admins can set `metadata.allowed_passthrough_routes` on a {entity}.")]
    PassthroughRoutesMetadata { entity: String },
    #[error("Only proxy admins can set `disable_global_guardrails` on a {entity}.")]
    GuardrailsDisabled { entity: String },
    #[error("metadata is not a JSON object, so {field} cannot move into it")]
    MetadataNotAnObject { field: String },
}

impl ManagementGuardError {
    #[must_use]
    pub const fn status_code(&self) -> Option<StatusCode> {
        match self {
            Self::NonFiniteSpend { .. }
            | Self::EmptyModelName
            | Self::ModelMaxBudget { .. }
            | Self::ModelBudgetDuration { .. }
            | Self::ModelRateLimit { .. } => Some(StatusCode::BAD_REQUEST),
            Self::BudgetDuration { status, .. } => Some(*status),
            Self::ServiceAccountAnalytics
            | Self::PassthroughRoutes { .. }
            | Self::PassthroughRoutesMetadata { .. }
            | Self::GuardrailsDisabled { .. } => Some(StatusCode::FORBIDDEN),
            Self::MetadataNotAnObject { .. } => None,
        }
    }
}
