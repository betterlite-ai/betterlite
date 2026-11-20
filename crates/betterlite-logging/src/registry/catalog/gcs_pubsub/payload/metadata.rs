use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::ptu_pricing::spillover::azure_spillover;

use super::NO_BODY_STORAGE;
use super::ProjectionError;
use crate::fields::payload::StandardLoggingPayload;
use crate::spend_metadata::projection::{SpendMetadataArguments, project_spend_logs_metadata};
use crate::spend_metadata::records::SpendLogsMetadata;
use crate::spend_metadata::router_metadata::router_metadata_for_spend_log;
use crate::spend_metadata::{as_object, optional_field};

// The provider whose OAuth client token the proxy forwards; every other provider reads as not forwarded.
const OAUTH_FORWARD_PROVIDERS: [&str; 1] = ["anthropic"];

// _get_spend_logs_metadata over the merged event metadata, with every enrichment argument the standard payload supplies
pub(super) fn clean_metadata(
    payload: &StandardLoggingPayload,
    merged: &JsonObject,
) -> Result<SpendLogsMetadata, ProjectionError> {
    let mut clean = project_spend_logs_metadata(
        Some(merged),
        metadata_arguments(payload, merged)?,
        &NO_BODY_STORAGE.sanitize,
    )?;
    // LiteLLM always writes additional_usage_values, and an event keeps no usage keys beyond the three totals
    clean.additional_usage_values = Some(Json::Object(JsonObject::new()));
    Ok(clean)
}

fn metadata_arguments(
    payload: &StandardLoggingPayload,
    merged: &JsonObject,
) -> Result<SpendMetadataArguments, ProjectionError> {
    let stored = &payload.metadata;
    let hidden = &payload.hidden_params;
    Ok(SpendMetadataArguments {
        applied_guardrails: stored.applied_guardrails.clone(),
        batch_models: hidden.batch_models.clone(),
        batch_successful_requests: hidden.batch_successful_requests,
        batch_failed_requests: hidden.batch_failed_requests,
        mcp_tool_call_metadata: optional_field(
            stored.mcp_tool_call_metadata.as_ref(),
            "mcp_tool_call_metadata",
            as_object,
        )?
        .cloned(),
        vector_store_request_metadata: stored.vector_store_request_metadata.clone(),
        guardrail_information: payload
            .guardrail_information
            .as_ref()
            .map(|information| {
                serde_json::to_value(information).and_then(serde_json::from_value::<Json>)
            })
            .transpose()
            .map_err(ProjectionError::Encode)?,
        usage_object: optional_field(stored.usage_object.as_ref(), "usage_object", as_object)?
            .cloned(),
        model_map_information: Some(payload.model_map_information.clone()),
        cold_storage_object_key: stored.cold_storage_object_key.clone(),
        litellm_overhead_time_ms: hidden.litellm_overhead_time_ms.map(|stamp| stamp.0),
        cost_breakdown: payload.cost_breakdown.clone(),
        litellm_call_id: payload.litellm_call_id.clone(),
        autorouter_savings: payload.autorouter_savings,
        autorouter_savings_estimate: payload.autorouter_savings_estimate.clone(),
        baseline_observation: payload.baseline_observation.clone(),
        router_metadata: router_metadata_for_spend_log(
            Some(merged),
            payload.model_group.as_deref(),
            Some(payload.model.as_str()),
            payload.custom_llm_provider.as_deref(),
            payload.litellm_call_id.as_deref(),
        ),
        azure_spillover: spillover(payload)?,
        used_client_oauth_token: oauth_token(merged, payload.custom_llm_provider.as_deref())?,
    })
}

// azure_spillover over the additional headers the standard payload kept; an event carries no response headers
fn spillover(payload: &StandardLoggingPayload) -> Result<Option<JsonObject>, ProjectionError> {
    let Some(headers) = payload.hidden_params.additional_headers.as_ref() else {
        return Ok(None);
    };
    let Some(found) = azure_spillover(None, Some(&text_headers(headers))) else {
        return Ok(None);
    };
    serde_json::to_value(&found)
        .and_then(serde_json::from_value::<JsonObject>)
        .map(Some)
        .map_err(ProjectionError::Encode)
}

// A header value is text; another type is not the header map's declared shape and cannot name a spillover header.
fn text_headers(fields: &JsonObject) -> BTreeMap<String, String> {
    fields
        .iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}

// resolve_used_client_oauth_token: only a stamped boolean counts, and it is forwarded for anthropic alone.
fn oauth_token(
    merged: &JsonObject,
    provider: Option<&str>,
) -> Result<Option<bool>, ProjectionError> {
    let stamped = optional_field(
        merged.get("used_client_oauth_token"),
        "used_client_oauth_token",
        as_bool,
    )?;
    let forwarded = provider.is_some_and(|name| OAUTH_FORWARD_PROVIDERS.contains(&name));
    Ok(stamped.map(|sent| sent && forwarded))
}

fn as_bool(value: &Json) -> Option<bool> {
    match value {
        Json::Bool(flag) => Some(*flag),
        Json::Null | Json::Number(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
