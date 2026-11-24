use betterlite_core::duration::error::DurationError;
use betterlite_db::budget_permission_rows::JsonData;
use rust_decimal::Decimal;

use crate::key_limit_policy::reads::KeyLimitReadError;
use crate::team_parent_limits::error::{RateLimit, RejectionDetail};

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum Entity {
    #[strum(to_string = "team")]
    Team,
    #[strum(to_string = "organization")]
    Organization,
}

// key_management_endpoints.py raises every ceiling rejection as HTTPException(status_code=400)
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("{field} must be a finite number. Received: {received}")]
    NonFinite {
        field: &'static str,
        received: String,
    },
    #[error(
        "{field} is over max limit set in config - user_value={user_value}; max_value={max_value}"
    )]
    Upperbound {
        field: &'static str,
        user_value: String,
        max_value: String,
    },
    #[error(
        "Allocated {limit} limit={allocated} + Key {limit} limit={requested} is greater than {entity} {limit} limit={ceiling}"
    )]
    Allocation {
        limit: RateLimit,
        entity: Entity,
        allocated: i128,
        requested: i64,
        ceiling: i64,
    },
    #[error("Project not found, project_id={project_id}")]
    ProjectNotFound { project_id: String },
    // the model name renders as JSON text, since Python's repr quotes it with apostrophes
    #[error(
        "Model {} not in project's allowed models. Project allowed models={allowed:?}. Project: {project_id}",
        json_text(.model)
    )]
    ProjectModel {
        model: JsonData,
        allowed: Vec<String>,
        project_id: String,
    },
    #[error(
        "Key max_budget ({requested}) exceeds project's max_budget ({ceiling}). Project: {project_id}"
    )]
    ProjectBudget {
        requested: Decimal,
        ceiling: Decimal,
        project_id: String,
    },
    #[error("Cannot assign a key to an organization without a user_id on the caller's token")]
    AssignWithoutUser,
    #[error("Caller is not a member of organization_id={organization_id}")]
    NotMember { organization_id: String },
}

// detail={"error": ...} for the guarded parameters and a bare string for the allocation and caller checks
impl Rejection {
    // the status each raised HTTPException carries in LiteLLM
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::ProjectNotFound { .. } => 404,
            Self::AssignWithoutUser | Self::NotMember { .. } => 403,
            Self::NonFinite { .. }
            | Self::Upperbound { .. }
            | Self::Allocation { .. }
            | Self::ProjectModel { .. }
            | Self::ProjectBudget { .. } => 400,
        }
    }

    #[must_use]
    pub fn detail(&self) -> RejectionDetail {
        let message = self.to_string();
        match self {
            Self::Allocation { .. } | Self::AssignWithoutUser | Self::NotMember { .. } => {
                RejectionDetail::Text(message)
            }
            Self::NonFinite { .. }
            | Self::Upperbound { .. }
            | Self::ProjectNotFound { .. }
            | Self::ProjectModel { .. }
            | Self::ProjectBudget { .. } => RejectionDetail::Error { error: message },
        }
    }
}

// a stored model name as the JSON string it is, not JsonData's variant debug
fn json_text(value: &JsonData) -> String {
    match value {
        JsonData::String(text) => format!("{text:?}"),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::Array(_)
        | JsonData::Object(_) => {
            format!("{value:?}")
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum KeyLimitError {
    #[error(transparent)]
    Rejected(#[from] Rejection),
    #[error("{field} must map model names to whole-number limits")]
    ModelLimits {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("{field} is not a duration")]
    Duration {
        field: &'static str,
        #[source]
        source: DurationError,
    },
    #[error(transparent)]
    Read(#[from] KeyLimitReadError),
    #[error("the organization has no organization_id to read its keys by")]
    MissingOrganization,
}
