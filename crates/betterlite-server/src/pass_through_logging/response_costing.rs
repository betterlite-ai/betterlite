use betterlite_core::cost::recording::PortFailure;
use betterlite_logging::logging_metadata::params::{LitellmParams, MetadataMap};
use betterlite_logging::logging_metadata::router::{RouterModelError, get_router_model_id};
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::pricing::{CUSTOM_PRICING_KEYS, takes_custom_pricing};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rust_decimal::Decimal;
use thiserror::Error;

use super::batch_attribution::optional_str;

pub struct ResponseCostRequest<'request> {
    pub response: &'request Json,
    pub model: &'request str,
    pub custom_llm_provider: Option<&'request str>,
    pub custom_pricing: Option<bool>,
    pub router_model_id: Option<&'request str>,
}

// B068's completion_cost, cut to the calls the pass-through leaves make; the proxy binds one implementation
pub trait ResponseCostService: Send + Sync {
    fn completion_cost(&self, request: &ResponseCostRequest<'_>) -> Result<Decimal, PortFailure>;
}

#[derive(Debug, Error)]
pub enum ResponseCostError {
    #[error("the response cost could not be computed: {0}")]
    Completion(#[source] PortFailure),
    #[error("the router model id could not be read: {0}")]
    Router(#[from] RouterModelError),
}

#[must_use]
pub fn cost_relevant_speed(request_body: Option<&JsonObject>) -> Option<String> {
    match request_body?.get("speed") {
        Some(Json::Text(speed)) => Some(speed.clone()),
        None | Some(_) => None,
    }
}

#[must_use]
pub fn resolve_costing_model(model: &str, call_log: &LoggingState) -> String {
    if !model.is_empty() && model != "unknown" {
        return model.to_owned();
    }
    let params = cost_params(call_log);
    let deployment_model = params.and_then(|fields| usable_model_in(fields.get("model")));
    if let Some(deployment) = deployment_model {
        return deployment.to_owned();
    }
    let group_model = params
        .and_then(|fields| fields.get("metadata"))
        .and_then(object_of)
        .and_then(|metadata| non_empty_text_in(metadata.get("model_group")));
    if let Some(model_group) = group_model {
        return model_group
            .strip_prefix("passthrough/")
            .unwrap_or(model_group)
            .to_owned();
    }
    model.to_owned()
}

pub fn compute_response_cost(
    service: &dyn ResponseCostService,
    response: &Json,
    model: &str,
    call_log: &LoggingState,
) -> Result<Decimal, ResponseCostError> {
    if is_cache_hit(call_log) {
        return Ok(Decimal::ZERO);
    }
    let provider = provider_of(call_log);
    let model_for_cost = prefixed_model(model, provider);
    let custom_pricing = uses_custom_pricing(&call_log.litellm_params);
    let router_id = get_router_model_id(&router_params(&call_log.litellm_params))?;
    let request = ResponseCostRequest {
        response,
        model: &model_for_cost,
        custom_llm_provider: provider,
        custom_pricing: Some(custom_pricing),
        router_model_id: router_id.as_deref(),
    };
    service
        .completion_cost(&request)
        .map_err(ResponseCostError::Completion)
}

#[must_use]
pub fn cost_partial_stream_or_zero(
    service: &dyn ResponseCostService,
    partial_response: &Json,
    model: &str,
    call_log: &LoggingState,
) -> Decimal {
    let resolved = resolve_costing_model(model, call_log);
    match compute_response_cost(service, partial_response, &resolved, call_log) {
        Ok(cost) => cost,
        Err(error) => {
            tracing::warn!(
                "Anthropic passthrough: could not cost the partial usage of an interrupted stream (model={model}): {error}"
            );
            Decimal::ZERO
        }
    }
}

fn cost_params(call_log: &LoggingState) -> Option<&JsonObject> {
    match call_log.model_call_details.get("litellm_params") {
        Some(Json::Object(fields)) => Some(fields),
        None | Some(_) => None,
    }
}

fn usable_model_in(value: Option<&Json>) -> Option<&str> {
    let model = text_in(value)?;
    if model.is_empty() || model == "unknown" {
        return None;
    }
    Some(model)
}

fn non_empty_text_in(value: Option<&Json>) -> Option<&str> {
    let text = text_in(value)?;
    if text.is_empty() {
        return None;
    }
    Some(text)
}

fn text_in(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        None | Some(_) => None,
    }
}

const fn object_of(value: &Json) -> Option<&JsonObject> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

fn is_cache_hit(call_log: &LoggingState) -> bool {
    match call_log.model_call_details.get("cache_hit") {
        Some(Json::Bool(true)) => true,
        None | Some(_) => false,
    }
}

fn provider_of(call_log: &LoggingState) -> Option<&str> {
    let provider = optional_str(call_log.model_call_details.get("custom_llm_provider")?)?;
    if provider.is_empty() {
        return None;
    }
    Some(provider)
}

fn prefixed_model(model: &str, provider: Option<&str>) -> String {
    let Some(prefix_source) = provider else {
        return model.to_owned();
    };
    let prefix = format!("{prefix_source}/");
    if model.starts_with(&prefix) {
        return model.to_owned();
    }
    format!("{prefix_source}/{model}")
}

// use_custom_pricing_for_model: the pricing fields on litellm_params, then each deployment model_info
fn uses_custom_pricing(litellm_params: &JsonObject) -> bool {
    if takes_custom_pricing(litellm_params) {
        return true;
    }
    for field in ["metadata", "litellm_metadata"] {
        let Some(Json::Object(metadata)) = litellm_params.get(field) else {
            continue;
        };
        let Some(Json::Object(model_info)) = metadata.get("model_info") else {
            continue;
        };
        let priced = CUSTOM_PRICING_KEYS.iter().any(|key| {
            model_info
                .get(*key)
                .is_some_and(|value| *value != Json::Null)
        });
        let off_peak = model_info
            .get("off_peak_pricing")
            .is_some_and(|value| *value != Json::Null);
        if priced || off_peak {
            return true;
        }
    }
    false
}

fn router_params(litellm_params: &JsonObject) -> LitellmParams {
    LitellmParams {
        metadata: fields_of(litellm_params.get("metadata")),
        litellm_metadata: fields_of(litellm_params.get("litellm_metadata")),
        litellm_session_id: None,
        litellm_trace_id: None,
        prompt_id: None,
        prompt_variables: None,
        langfuse_masking_function: None,
    }
}

fn fields_of(value: Option<&Json>) -> Option<MetadataMap> {
    match value {
        Some(Json::Object(fields)) => Some(fields.clone()),
        None | Some(_) => None,
    }
}

mod tests;
