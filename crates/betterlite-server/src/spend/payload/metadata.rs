use std::collections::BTreeMap;

use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::spend_metadata::optional_object_field;
use betterlite_logging::spend_metadata::projection::{
    SpendMetadataArguments, project_spend_logs_metadata,
};
use betterlite_logging::spend_metadata::records::{SpendLogsMetadata, SpendLogsRouterMetadata};
use betterlite_models::call_helpers::metadata::{
    CallParams, InternalMetadata, MetadataMap, stamped_oauth_token,
};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::ptu_pricing::spillover::{AzureSpillover, azure_spillover};

use super::identifiers::Attribution;
use super::{Context, SpendPayloadError, SpendPayloadSettings, as_object};

// The provider whose OAuth client token the proxy forwards; every other provider reads as not forwarded.
const OAUTH_FORWARD_PROVIDERS: [&str; 1] = ["anthropic"];

// _get_spend_logs_metadata over the persisted metadata, with every enrichment argument the standard payload supplies.
pub(crate) fn project(
    context: &Context<'_>,
    settings: &SpendPayloadSettings,
    attribution: &Attribution,
) -> Result<SpendLogsMetadata, SpendPayloadError> {
    let arguments = arguments(context, attribution)?;
    Ok(project_spend_logs_metadata(
        Some(&attribution.persisted_metadata),
        arguments,
        &settings.storage.sanitize,
    )?)
}

fn arguments(
    context: &Context<'_>,
    attribution: &Attribution,
) -> Result<SpendMetadataArguments, SpendPayloadError> {
    let standard = context.standard.as_ref();
    let stored = standard.and_then(|source| source.metadata.as_ref());
    let hidden = standard.and_then(|source| source.hidden_params.as_ref());
    let guardrail_information = match standard {
        Some(source) => source.guardrail_information.clone(),
        None => context
            .metadata
            .get("standard_logging_guardrail_information")
            .cloned(),
    };
    let router_metadata = router_metadata(context, attribution)?;
    let oauth = used_client_oauth_token(context, attribution.provider.as_deref())?;
    let spillover = spillover(context)?;
    Ok(SpendMetadataArguments {
        applied_guardrails: stored.and_then(|fields| fields.applied_guardrails.clone()),
        batch_models: hidden.and_then(|fields| fields.batch_models.clone()),
        batch_successful_requests: hidden.and_then(|fields| fields.batch_successful_requests),
        batch_failed_requests: hidden.and_then(|fields| fields.batch_failed_requests),
        mcp_tool_call_metadata: stored.and_then(|fields| fields.mcp_tool_call_metadata.clone()),
        vector_store_request_metadata: stored
            .and_then(|fields| fields.vector_store_request_metadata.clone()),
        guardrail_information,
        usage_object: stored.and_then(|fields| fields.usage_object.clone()),
        model_map_information: standard.and_then(|source| source.model_map_information.clone()),
        cold_storage_object_key: stored.and_then(|fields| fields.cold_storage_object_key.clone()),
        litellm_overhead_time_ms: hidden.and_then(|fields| fields.litellm_overhead_time_ms),
        cost_breakdown: standard.and_then(|source| source.cost_breakdown.clone()),
        litellm_call_id: attribution.call_id.clone(),
        autorouter_savings: standard.and_then(|source| source.autorouter_savings),
        autorouter_savings_estimate: standard
            .and_then(|source| source.autorouter_savings_estimate.clone()),
        baseline_observation: standard
            .and_then(|source| source.autorouter_baseline_observation.clone()),
        router_metadata,
        azure_spillover: spillover,
        used_client_oauth_token: oauth,
    })
}

// Only an internally routed model stamps router provenance, and an empty spelling reads as unknown.
fn router_metadata(
    context: &Context<'_>,
    attribution: &Attribution,
) -> Result<Option<SpendLogsRouterMetadata>, SpendPayloadError> {
    let model_info = optional_object_field(context.metadata.get("model_info"), "model_info")?;
    let Some(fields) = model_info else {
        return Ok(None);
    };
    if !matches!(fields.get("internal_router_model"), Some(Json::Bool(true))) {
        return Ok(None);
    }
    Ok(Some(SpendLogsRouterMetadata {
        requested_model: nonempty(&attribution.model_group),
        selected_model: nonempty(&attribution.model_name),
        selected_provider: attribution.provider.clone().filter(|name| !name.is_empty()),
        router_correlation_id: attribution.call_id.clone(),
    }))
}

fn nonempty(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_owned())
}

// resolve_used_client_oauth_token: only a stamped boolean counts, and it is forwarded for anthropic alone.
fn used_client_oauth_token(
    context: &Context<'_>,
    provider: Option<&str>,
) -> Result<Option<bool>, SpendPayloadError> {
    let params = call_params(&context.litellm_params)?;
    let Some(stamped) = stamped_oauth_token(params.metadata.as_ref(), Some(&params)) else {
        return Ok(None);
    };
    let Json::Bool(sent) = stamped else {
        return Ok(None);
    };
    let forwarded = provider.is_some_and(|name| OAUTH_FORWARD_PROVIDERS.contains(&name));
    Ok(Some(*sent && forwarded))
}

// The two metadata buckets a litellm_params record carries, which the oauth stamp is read from.
fn call_params(litellm_params: &JsonObject) -> Result<CallParams, SpendPayloadError> {
    let metadata = bucket(litellm_params, "metadata")?;
    let litellm_metadata = bucket(litellm_params, "litellm_metadata")?;
    Ok(CallParams {
        metadata,
        litellm_metadata: litellm_metadata.map(|entries| InternalMetadata { entries }),
    })
}

fn bucket(
    litellm_params: &JsonObject,
    field: &'static str,
) -> Result<Option<MetadataMap>, SpendPayloadError> {
    match litellm_params.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => Ok(Some(fields.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(SpendPayloadError::FieldType { field })
        }
    }
}

// azure_spillover over the response headers, then the llm_provider-prefixed ones the standard payload kept.
fn spillover(context: &Context<'_>) -> Result<Option<JsonObject>, SpendPayloadError> {
    let response_headers = headers_of(context.kwargs.get("response_headers"));
    let additional_headers = context
        .standard
        .as_ref()
        .and_then(|source| source.hidden_params.as_ref())
        .and_then(|fields| fields.additional_headers.as_ref())
        .map(text_headers);
    let Some(found) = azure_spillover(response_headers.as_ref(), additional_headers.as_ref())
    else {
        return Ok(None);
    };
    Ok(Some(record(&found)?))
}

// Only a mapping carries headers; any other value at the kwarg reads as no headers at all.
fn headers_of(value: Option<&Json>) -> Option<BTreeMap<String, String>> {
    let fields = value.and_then(as_object)?;
    Some(text_headers(fields))
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

fn record(spillover: &AzureSpillover) -> Result<JsonObject, SpendPayloadError> {
    serde_json::to_value(spillover)
        .and_then(serde_json::from_value)
        .map_err(|source| SpendPayloadError::Json {
            record: "azure_spillover",
            source: PrivateJsonError::Encode(source),
        })
}
