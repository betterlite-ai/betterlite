use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde::Deserialize;

use super::records::{SpendLogsMetadata, SpendLogsRouterMetadata};
use super::redaction::{is_non_secret_key_value, redact_logged_api_key};
use super::{SpendMetadataError, metadata_text, nonempty_text};
use crate::logging_metadata::decode::decode;
use crate::spend_bodies::vector_store::redact_vector_store;
use crate::spend_sanitize::SpendSanitizePolicy;
use crate::spend_sanitize::guardrails::sanitize_guardrail_information;
use crate::spend_sanitize::value::SpendValue;

const RECORD: &str = "spend log metadata";
const USER_API_KEY_FIELD: &str = "user_api_key";
const OVERRIDDEN_FIELDS: [&str; 13] = [
    "applied_guardrails",
    "batch_models",
    "batch_successful_requests",
    "batch_failed_requests",
    "mcp_tool_call_metadata",
    "vector_store_request_metadata",
    "guardrail_information",
    "usage_object",
    "model_map_information",
    "cold_storage_object_key",
    "litellm_overhead_time_ms",
    "cost_breakdown",
    "litellm_call_id",
];
const STAMPED_FIELDS: [&str; 6] = [
    "router_metadata",
    "azure_spillover",
    "autorouter_savings",
    "autorouter_savings_estimate",
    "autorouter_baseline_observation",
    "used_client_oauth_token",
];

// The explicit arguments _get_spend_logs_metadata takes besides the metadata mapping.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct SpendMetadataArguments {
    pub applied_guardrails: Option<Vec<String>>,
    pub batch_models: Option<Vec<String>>,
    pub batch_successful_requests: Option<i64>,
    pub batch_failed_requests: Option<i64>,
    pub mcp_tool_call_metadata: Option<JsonObject>,
    pub vector_store_request_metadata: Option<Vec<Json>>,
    pub guardrail_information: Option<Json>,
    pub usage_object: Option<JsonObject>,
    pub model_map_information: Option<JsonObject>,
    pub cold_storage_object_key: Option<String>,
    pub litellm_overhead_time_ms: Option<f64>,
    pub cost_breakdown: Option<JsonObject>,
    pub litellm_call_id: Option<String>,
    pub autorouter_savings: Option<f64>,
    pub autorouter_savings_estimate: Option<JsonObject>,
    pub baseline_observation: Option<String>,
    pub router_metadata: Option<SpendLogsRouterMetadata>,
    pub azure_spillover: Option<JsonObject>,
    pub used_client_oauth_token: Option<bool>,
}

pub fn project_spend_logs_metadata(
    metadata: Option<&JsonObject>,
    arguments: SpendMetadataArguments,
    policy: &SpendSanitizePolicy,
) -> Result<SpendLogsMetadata, SpendMetadataError> {
    let Some(metadata) = metadata else {
        return Ok(absent_metadata(arguments));
    };
    let mut fields = metadata.clone();
    let raw_key = fields.shift_remove(USER_API_KEY_FIELD);
    for field in OVERRIDDEN_FIELDS {
        fields.shift_remove(field);
    }
    for field in STAMPED_FIELDS {
        fields.shift_remove(field);
    }
    let mut record: SpendLogsMetadata = decode(RECORD, fields)?;
    // The metadata branch emits every annotation, so the six NotRequired agent fields are present and null
    record.actor_agent_id = Some(record.actor_agent_id.flatten());
    record.target_agent_id = Some(record.target_agent_id.flatten());
    record.billing_agent_id = Some(record.billing_agent_id.flatten());
    record.agent_execution_mode = Some(record.agent_execution_mode.flatten());
    record.verified_human_user_id = Some(record.verified_human_user_id.flatten());
    record.routing_decision = Some(record.routing_decision.flatten());
    record.hashed_user_api_key = redact_logged_api_key(
        nonempty_text(raw_key.as_ref()),
        trusted_redaction(metadata, raw_key.as_ref()),
        metadata_text(metadata, "user_api_key_alias"),
    );
    record.applied_guardrails = arguments.applied_guardrails;
    record.batch_models = arguments.batch_models;
    record.batch_successful_requests = arguments.batch_successful_requests;
    record.batch_failed_requests = arguments.batch_failed_requests;
    record.mcp_tool_call_metadata = arguments.mcp_tool_call_metadata.map(Json::Object);
    let mut vector_requests = arguments.vector_store_request_metadata;
    if let Some(requests) = vector_requests.as_mut() {
        redact_vector_store(requests, policy.store_bodies);
    }
    record.vector_store_request_metadata = vector_requests;
    record.guardrail_information = match arguments.guardrail_information {
        Some(information) => Some(sanitize_guardrail_information(
            &SpendValue::from(&information),
            policy,
        )),
        None => None,
    };
    record.usage_object = arguments.usage_object.map(Json::Object);
    record.model_map_information = arguments.model_map_information.map(Json::Object);
    record.cold_storage_object_key = arguments.cold_storage_object_key;
    record.litellm_overhead_time_ms = arguments.litellm_overhead_time_ms;
    record.cost_breakdown = arguments.cost_breakdown.map(Json::Object);
    record.litellm_call_id = arguments.litellm_call_id;
    record.autorouter_savings = arguments.autorouter_savings;
    record.autorouter_savings_estimate = arguments.autorouter_savings_estimate.map(Json::Object);
    record.baseline_observation = arguments.baseline_observation;
    record.router_metadata = arguments.router_metadata;
    record.azure_spillover = arguments.azure_spillover.map(Json::Object);
    record.used_client_oauth_token = arguments.used_client_oauth_token;
    Ok(record)
}

// The metadata=None constructor: success status, no agent fields, and only the enrichment arguments the source keeps.
fn absent_metadata(arguments: SpendMetadataArguments) -> SpendLogsMetadata {
    let mut record = SpendLogsMetadata::default();
    record.status = Some("success".to_owned());
    record.cold_storage_object_key = arguments.cold_storage_object_key;
    record.litellm_call_id = arguments.litellm_call_id;
    record.autorouter_savings = arguments.autorouter_savings;
    record.autorouter_savings_estimate = arguments.autorouter_savings_estimate.map(Json::Object);
    record.baseline_observation = arguments.baseline_observation;
    record.router_metadata = arguments.router_metadata;
    record.azure_spillover = arguments.azure_spillover.map(Json::Object);
    record.used_client_oauth_token = arguments.used_client_oauth_token;
    record
}

// The trusted hash must be a non-secret string equal to the raw metadata key before it disables hashing.
fn trusted_redaction(metadata: &JsonObject, raw_key: Option<&Json>) -> bool {
    let Some(trusted) = metadata_text(metadata, "user_api_key_hash") else {
        return false;
    };
    let alias = metadata_text(metadata, "user_api_key_alias");
    is_non_secret_key_value(trusted, alias)
        && matches!(raw_key, Some(Json::Text(key)) if key.as_str() == trusted)
}

#[path = "projection/tests.rs"]
mod tests;
