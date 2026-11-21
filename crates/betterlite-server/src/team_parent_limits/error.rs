use std::fmt;

use axum::http::StatusCode;
use betterlite_auth::auth_user_lookup::error::UserLookupError;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::DbError;
use betterlite_db::team_repository::repository::TeamQueryError;
use rust_decimal::Decimal;
use serde::Serialize;

// LiteLLM raises every ceiling rejection as HTTPException(status_code=400)
pub const REJECTION_STATUS: StatusCode = StatusCode::BAD_REQUEST;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateLimit {
    Rpm,
    Tpm,
}

impl fmt::Display for RateLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rpm => "RPM",
            Self::Tpm => "TPM",
        })
    }
}

// the allocation text is LiteLLM's own f-string, since it carries only integers
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error(
        "Team max_budget ({requested}) exceeds organization's max_budget ({ceiling}). Organization: {organization}"
    )]
    OrganizationBudget {
        requested: Decimal,
        ceiling: Decimal,
        organization: String,
    },
    #[error(
        "Model {model:?} not in organization's allowed models. Organization allowed models={allowed:?}. Organization: {organization}"
    )]
    OrganizationModel {
        model: JsonData,
        allowed: Vec<String>,
        organization: String,
    },
    #[error(
        "Team tpm_limit ({requested}) exceeds organization's tpm_limit ({ceiling}). Organization: {organization}"
    )]
    OrganizationTpm {
        requested: i64,
        ceiling: i64,
        organization: String,
    },
    #[error(
        "Team rpm_limit ({requested}) exceeds organization's rpm_limit ({ceiling}). Organization: {organization}"
    )]
    OrganizationRpm {
        requested: i64,
        ceiling: i64,
        organization: String,
    },
    #[error(
        "Allocated {limit} limit={allocated} + Team {limit} limit={requested} is greater than organization {limit} limit={ceiling}"
    )]
    Allocation {
        limit: RateLimit,
        allocated: i128,
        requested: i64,
        ceiling: i64,
    },
    #[error("max budget higher than user max. User max budget={ceiling}. User role={role}")]
    UserBudget { ceiling: Decimal, role: String },
    #[error("Model not in allowed user models. User allowed models={allowed:?}. User id={user}")]
    UserModel {
        allowed: Vec<JsonData>,
        user: String,
    },
    #[error("tpm limit higher than user max. User tpm limit={ceiling}. User role={role}")]
    UserTpm { ceiling: i64, role: String },
    #[error("rpm limit higher than user max. User rpm limit={ceiling}. User role={role}")]
    UserRpm { ceiling: i64, role: String },
}

// detail={"error": ...} for the direct checks and a bare string for the allocation checks
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum RejectionDetail {
    Error { error: String },
    Text(String),
}

impl Rejection {
    #[must_use]
    pub fn detail(&self) -> RejectionDetail {
        let message = self.to_string();
        match self {
            Self::Allocation { .. } => RejectionDetail::Text(message),
            Self::OrganizationBudget { .. }
            | Self::OrganizationModel { .. }
            | Self::OrganizationTpm { .. }
            | Self::OrganizationRpm { .. }
            | Self::UserBudget { .. }
            | Self::UserModel { .. }
            | Self::UserTpm { .. }
            | Self::UserRpm { .. } => RejectionDetail::Error { error: message },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TeamLimitError {
    #[error(transparent)]
    Rejected(#[from] Rejection),
    #[error("{field} must map model names to whole-number limits")]
    ModelLimits {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the caller's user_id, user_role, models and limits cannot be read")]
    Caller(#[source] serde_json::Error),
    #[error("the organization has no organization_id to read its teams by")]
    MissingOrganization,
    #[error(transparent)]
    Database(#[from] DbError),
    #[error(transparent)]
    TeamQuery(#[from] TeamQueryError),
    #[error(transparent)]
    UserLookup(#[from] UserLookupError),
}
