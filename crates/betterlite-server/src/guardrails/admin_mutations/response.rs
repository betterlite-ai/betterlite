use betterlite_config::sections::guardrail_params::opaque::OpaqueParams;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_config::sections::guardrail_params::records::Guardrail;
use betterlite_db::budget_permission_rows::Timestamp;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::value::RawValue;

use super::review_rows::ReviewRow;

// LiteLLM answers the request guardrail with the new id, so only the keys the client sent come back
#[derive(Serialize)]
pub(super) struct CreatedGuardrail {
    pub(super) guardrail_id: String,
    pub(super) guardrail_name: String,
    pub(super) litellm_params: LitellmParams,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) guardrail_info: Option<Box<RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) policy_template: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) updated_at: Option<DateTime<Utc>>,
}

// Prisma's update returns every column, so the review columns ride along with the registry's row
#[derive(Serialize)]
pub(super) struct StoredGuardrail {
    pub(super) guardrail_id: Option<String>,
    pub(super) guardrail_name: String,
    pub(super) litellm_params: LitellmParams,
    pub(super) guardrail_info: Option<OpaqueParams>,
    pub(super) team_id: Option<String>,
    pub(super) created_at: Option<DateTime<Utc>>,
    pub(super) updated_at: Option<DateTime<Utc>>,
    pub(super) status: String,
    pub(super) submitted_at: Option<DateTime<Utc>>,
    pub(super) reviewed_at: Option<DateTime<Utc>>,
}

impl StoredGuardrail {
    pub(super) fn from_rows(guardrail: Guardrail, review: ReviewRow) -> Self {
        Self {
            guardrail_id: guardrail.guardrail_id,
            guardrail_name: guardrail.guardrail_name,
            litellm_params: guardrail.litellm_params,
            guardrail_info: guardrail.guardrail_info,
            team_id: guardrail.team_id,
            created_at: guardrail.created_at,
            updated_at: guardrail.updated_at,
            status: review.status,
            submitted_at: review.submitted_at.map(Timestamp::utc),
            reviewed_at: review.reviewed_at.map(Timestamp::utc),
        }
    }
}

#[derive(Serialize)]
pub(super) struct SubmissionDecision {
    pub(super) guardrail_id: String,
    pub(super) status: &'static str,
    pub(super) message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) warning: Option<String>,
}
