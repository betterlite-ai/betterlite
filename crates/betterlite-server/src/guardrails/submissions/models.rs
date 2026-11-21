use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_pyhost::json_values::JsonObject;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::{RegisterBodyError, SubmissionError};

pub(super) const ACTIVE: &str = "active";
pub(super) const PENDING_REVIEW: &str = "pending_review";
pub(super) const REJECTED: &str = "rejected";
pub(super) const GENERIC_GUARDRAIL_API: &str = "generic_guardrail_api";
pub(super) const GUARDRAIL: &str = "guardrail";
pub(super) const API_BASE: &str = "api_base";
pub(super) const MODE: &str = "mode";
pub(super) const DESCRIPTION: &str = "description";
pub(super) const SUBMITTED_BY_USER_ID: &str = "submitted_by_user_id";
pub(super) const SUBMITTED_BY_EMAIL: &str = "submitted_by_email";
pub(super) const TEAM_GUARDRAIL: &str = "team_guardrail";

#[derive(Debug, Deserialize)]
pub struct RegisterGuardrailRequest {
    pub guardrail_name: String,
    pub litellm_params: BTreeMap<String, GuardrailValue>,
    #[serde(default)]
    pub guardrail_info: Option<BTreeMap<String, GuardrailValue>>,
    #[serde(default)]
    pub team_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterGuardrailResponse {
    pub guardrail_id: String,
    pub guardrail_name: String,
    pub status: String,
    pub submitted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct GuardrailSubmissionItem {
    pub guardrail_id: String,
    pub guardrail_name: String,
    pub status: String,
    pub team_id: Option<String>,
    pub team_guardrail: bool,
    pub litellm_params: Option<JsonObject>,
    pub guardrail_info: Option<JsonObject>,
    pub submitted_by_user_id: Option<String>,
    pub submitted_by_email: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct GuardrailSubmissionSummary {
    pub total: usize,
    pub pending_review: usize,
    pub active: usize,
    pub rejected: usize,
}

#[derive(Debug, Serialize)]
pub struct ListGuardrailSubmissionsResponse {
    pub submissions: Vec<GuardrailSubmissionItem>,
    pub summary: GuardrailSubmissionSummary,
}

// the route's only read of the body bytes; FastAPI validates the body before the handler runs
pub fn decode_register_body(body: &[u8]) -> Result<RegisterGuardrailRequest, SubmissionError> {
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let request = serde_path_to_error::deserialize(&mut deserializer).map_err(|source| {
        let issue = RegisterBodyError::serde_at(body, &source);
        SubmissionError::Body(Box::new(issue))
    })?;
    deserializer
        .end()
        .map_err(|_trailing| SubmissionError::Body(Box::new(RegisterBodyError::invalid_json())))?;
    Ok(request)
}
