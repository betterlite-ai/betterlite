use betterlite_core::api::media_relay_logging::spend::decimal_json;
use betterlite_logging::end_user::extraction::{IdSettings, request_end_user_id};
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;

use super::response_costing::{ResponseCostError, ResponseCostRequest, ResponseCostService};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResponseLoggingKwargs {
    pub response_cost: Option<Decimal>,
    pub model: Option<String>,
    pub litellm_params: JsonObject,
    pub standard_logging_object: Option<Json>,
    pub passthrough_request_body: Option<JsonObject>,
}

pub struct ResponseLoggingInput<'input> {
    pub response: &'input mut Json,
    pub model: &'input str,
    pub kwargs: ResponseLoggingKwargs,
    pub end_user: Option<&'input str>,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub call_log: &'input mut LoggingState,
}

#[derive(Debug, Error)]
enum ResponsePayloadError {
    #[error(transparent)]
    Cost(#[from] ResponseCostError),
    #[error("the standard logging object could not be built")]
    StandardObject,
    #[error("the standard logging object could not be serialized: {0}")]
    Encoding(#[source] serde_json::Error),
    #[error("the response body is not a JSON object")]
    ResponseNotObject,
}

#[must_use]
pub fn end_user_from_request_body(
    request_body: Option<&JsonObject>,
    settings: &IdSettings<'_>,
) -> Option<String> {
    let body = request_body?;
    if body.is_empty() {
        return None;
    }
    request_end_user_id(body, None, settings)
}

#[must_use]
// ast-grep-ignore: no-long-names - B3650 and B3634 call this fixed name from the base handler
pub fn create_response_logging_payload(
    cost: &dyn ResponseCostService,
    builder: &dyn StandardPayloadBuilder,
    mut input: ResponseLoggingInput<'_>,
) -> ResponseLoggingKwargs {
    if let Err(error) = run_steps(cost, builder, &mut input) {
        tracing::error!("Error creating LLM passthrough response logging payload: {error}");
    }
    input.kwargs
}

fn run_steps(
    cost: &dyn ResponseCostService,
    builder: &dyn StandardPayloadBuilder,
    input: &mut ResponseLoggingInput<'_>,
) -> Result<(), ResponsePayloadError> {
    apply_cost(cost, input)?;
    apply_user(input)?;
    apply_standard_object(builder, input)?;
    apply_identity(input)
}

fn apply_cost(
    cost: &dyn ResponseCostService,
    input: &mut ResponseLoggingInput<'_>,
) -> Result<(), ResponsePayloadError> {
    let request = ResponseCostRequest {
        response: &*input.response,
        model: input.model,
        custom_llm_provider: None,
        custom_pricing: None,
        router_model_id: None,
    };
    let response_cost = cost.completion_cost(&request)?;
    input.kwargs.response_cost = Some(response_cost);
    input.kwargs.model = Some(input.model.to_owned());
    input
        .call_log
        .model_call_details
        .insert("response_cost".to_owned(), decimal_json(response_cost));
    Ok(())
}

fn apply_user(input: &mut ResponseLoggingInput<'_>) -> Result<(), ResponsePayloadError> {
    let carries_body = input
        .kwargs
        .passthrough_request_body
        .as_ref()
        .is_some_and(|request_body| !request_body.is_empty());
    if !carries_body {
        return Ok(());
    }
    let Some(user) = input.end_user.filter(|user| !user.is_empty()) else {
        return Ok(());
    };
    let mut body = IndexMap::new();
    body.insert("user".to_owned(), Json::Text(user.to_owned()));
    let mut request = IndexMap::new();
    request.insert("body".to_owned(), Json::Object(body));
    let replacement = Json::Object(request);
    input
        .kwargs
        .litellm_params
        .insert("proxy_server_request".to_owned(), replacement.clone());
    input
        .call_log
        .litellm_params
        .insert("proxy_server_request".to_owned(), replacement);
    Ok(())
}

fn apply_standard_object(
    builder: &dyn StandardPayloadBuilder,
    input: &mut ResponseLoggingInput<'_>,
) -> Result<(), ResponsePayloadError> {
    input.call_log.model = input.model.to_owned();
    let built = builder.build(
        &*input.call_log,
        &*input.response,
        input.start_time,
        input.end_time,
    );
    let Some(payload) = built else {
        return Err(ResponsePayloadError::StandardObject);
    };
    let object = payload_json(&payload)?;
    let pretty = serde_json::to_string_pretty(&object).map_err(ResponsePayloadError::Encoding)?;
    tracing::debug!("standard_logging_object= {pretty}");
    input.kwargs.standard_logging_object = Some(object);
    Ok(())
}

fn apply_identity(input: &mut ResponseLoggingInput<'_>) -> Result<(), ResponsePayloadError> {
    let model = input.model.to_owned();
    let call_id = input.call_log.litellm_call_id.clone();
    let Json::Object(fields) = &mut *input.response else {
        return Err(ResponsePayloadError::ResponseNotObject);
    };
    fields.insert("id".to_owned(), Json::Text(call_id));
    fields.insert("model".to_owned(), Json::Text(model.clone()));
    input
        .call_log
        .model_call_details
        .insert("model".to_owned(), Json::Text(model));
    Ok(())
}

fn payload_json(payload: &StandardLoggingPayload) -> Result<Json, ResponsePayloadError> {
    serde_json::to_value(payload)
        .and_then(serde_json::from_value::<Json>)
        .map_err(ResponsePayloadError::Encoding)
}

mod tests;
