use std::collections::BTreeSet;
use std::str::FromStr as _;

use betterlite_config::sections::settings::JsonValue;
use betterlite_core::routes::CallType;
use betterlite_openai::openai_base_config::Object as JsonMap;
use rust_decimal::Decimal;
use serde_json::Number;
use thiserror::Error;

use super::ports::{FailureTokenCounter, RouteCallTypes, TokenCountError};

pub mod deployment;

pub const INPUT_ESTIMABLE_CALL_TYPES: [&str; 22] = [
    "completion",
    "acompletion",
    "text_completion",
    "atext_completion",
    "anthropic_messages",
    "aanthropic_messages",
    "responses",
    "aresponses",
    "embedding",
    "aembedding",
    "moderation",
    "amoderation",
    "image_generation",
    "aimage_generation",
    "speech",
    "aspeech",
    "rerank",
    "arerank",
    "generate_content",
    "agenerate_content",
    "generate_content_stream",
    "agenerate_content_stream",
];

pub const NO_UPSTREAM_LLM_CALL_KEY: &str = "litellm_no_upstream_llm_call";
pub const PASS_THROUGH_CALL_TYPE: &str = "pass_through_endpoint";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelCallDetails {
    pub start_time: Option<JsonValue>,
    pub first_api_call_start_time: Option<JsonValue>,
    pub call_type: Option<String>,
    pub combined_usage_object: Option<JsonValue>,
    pub response_cost: Option<Decimal>,
    pub standard_logging_object: Option<JsonValue>,
    pub model: Option<String>,
    pub messages: Option<Vec<JsonValue>>,
    pub optional_params: Option<JsonMap>,
    pub no_upstream_llm_call: bool,
    pub cache_hit: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailureUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
}

impl FailureUsage {
    fn to_value(self) -> JsonValue {
        let mut usage = JsonMap::new();
        usage.insert(
            "prompt_tokens".to_owned(),
            JsonValue::from(self.prompt_tokens),
        );
        usage.insert(
            "completion_tokens".to_owned(),
            JsonValue::from(self.completion_tokens),
        );
        usage.insert(
            "total_tokens".to_owned(),
            JsonValue::from(self.total_tokens),
        );
        JsonValue::Object(usage)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FailureFields {
    pub start_time: Option<JsonValue>,
    pub first_api_call_start_time: Option<JsonValue>,
    pub call_type: Option<JsonValue>,
    pub combined_usage_object: Option<JsonValue>,
    pub response_cost: Option<JsonValue>,
    pub standard_logging_object: Option<JsonValue>,
}

impl FailureFields {
    pub fn apply(self, request_data: &mut JsonMap) {
        for (name, value) in [
            ("start_time", self.start_time),
            ("first_api_call_start_time", self.first_api_call_start_time),
            ("call_type", self.call_type),
            ("combined_usage_object", self.combined_usage_object),
            ("response_cost", self.response_cost),
            ("standard_logging_object", self.standard_logging_object),
        ] {
            if let Some(present) = value {
                request_data.insert(name.to_owned(), present);
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum LiftError {
    #[error("the lifted response cost is not a JSON number")]
    Cost(#[source] serde_json::Error),
}

pub async fn failure_fields_to_lift(
    details: &ModelCallDetails,
    request_body: &JsonMap,
    counter: &dyn FailureTokenCounter,
) -> Result<FailureFields, LiftError> {
    let dispatched = details.first_api_call_start_time.is_some();
    let usage = failure_usage_to_lift(details, request_body, dispatched, counter).await;
    let response_cost = match &usage {
        Some((_, cost)) => Some(decimal_value(*cost)?),
        None => None,
    };
    let call_type = details.call_type.clone().map(JsonValue::String);
    Ok(FailureFields {
        start_time: details.start_time.clone(),
        first_api_call_start_time: details.first_api_call_start_time.clone(),
        call_type,
        combined_usage_object: usage.map(|(value, _)| value),
        response_cost,
        standard_logging_object: details.standard_logging_object.clone(),
    })
}

pub async fn failure_usage_to_lift(
    details: &ModelCallDetails,
    request_body: &JsonMap,
    dispatched: bool,
    counter: &dyn FailureTokenCounter,
) -> Option<(JsonValue, Decimal)> {
    if let Some(recovered) = &details.combined_usage_object {
        return Some((
            recovered.clone(),
            details.response_cost.unwrap_or(Decimal::ZERO),
        ));
    }
    if !dispatched || details.no_upstream_llm_call {
        return None;
    }
    let call_type = details.call_type.as_deref()?;
    if !INPUT_ESTIMABLE_CALL_TYPES.contains(&call_type) {
        return None;
    }
    let dispatched_system = details.optional_params.as_ref().and_then(|params| {
        params
            .get("system")
            .filter(|value| !value.is_null())
            .or_else(|| params.get("instructions").filter(|value| !value.is_null()))
    });
    let system_input = dispatched_system
        .or_else(|| request_body.get("system").filter(|value| !value.is_null()))
        .or_else(|| {
            request_body
                .get("instructions")
                .filter(|value| !value.is_null())
        });
    let usage = estimate_failure_usage(
        details.model.as_deref().unwrap_or_default(),
        details.messages.as_deref(),
        system_input,
        counter,
    )
    .await?;
    Some((usage.to_value(), Decimal::ZERO))
}

pub async fn estimate_failure_usage(
    model: &str,
    request_input: Option<&[JsonValue]>,
    system_input: Option<&JsonValue>,
    counter: &dyn FailureTokenCounter,
) -> Option<FailureUsage> {
    let input_tokens = count_request_input_tokens(model, request_input, system_input, counter)
        .await
        .ok()?;
    if input_tokens <= 0 {
        return None;
    }
    Some(FailureUsage {
        prompt_tokens: input_tokens,
        completion_tokens: 0,
        total_tokens: input_tokens,
    })
}

pub async fn count_request_input_tokens(
    model: &str,
    request_input: Option<&[JsonValue]>,
    system_input: Option<&JsonValue>,
    counter: &dyn FailureTokenCounter,
) -> Result<i64, TokenCountError> {
    let system_text = system_input.map_or_else(String::new, system_prompt_text);
    let system_tokens = if system_text.is_empty() {
        0
    } else {
        counter.count_text(model, &system_text).await?
    };
    let Some(entries) = request_input else {
        return Ok(system_tokens);
    };
    if entries.is_empty() {
        return Ok(system_tokens);
    }
    let input_tokens = if entries.iter().all(JsonValue::is_string) {
        let joined: String = entries.iter().filter_map(JsonValue::as_str).collect();
        counter.count_text(model, &joined).await?
    } else {
        counter.count_messages(model, entries).await?
    };
    system_tokens
        .checked_add(input_tokens)
        .ok_or(TokenCountError)
}

#[must_use]
pub fn system_prompt_text(system_input: &JsonValue) -> String {
    if let Some(text) = system_input.as_str() {
        return text.to_owned();
    }
    let Some(blocks) = system_input.as_array() else {
        return String::new();
    };
    blocks.iter().map(prompt_block_text).collect()
}

#[must_use]
pub fn prompt_block_text(block: &JsonValue) -> String {
    if let Some(text) = block.as_str() {
        return text.to_owned();
    }
    block
        .get("text")
        .and_then(JsonValue::as_str)
        .map_or_else(String::new, str::to_owned)
}

pub fn call_type_for_route(route: Option<&str>, routes: &dyn RouteCallTypes) -> Option<CallType> {
    let call_types = routes.call_types_for(route?)?;
    let mut operations = BTreeSet::new();
    for call_type in &call_types {
        let name = call_type_name(*call_type)?;
        operations.insert(name.strip_prefix('a').unwrap_or(&name).to_owned());
    }
    if operations.len() != 1 {
        return None;
    }
    call_types.first().copied()
}

#[must_use]
pub fn call_type_name(call_type: CallType) -> Option<String> {
    match serde_json::to_value(call_type) {
        Ok(JsonValue::String(name)) => Some(name),
        _ => None,
    }
}

fn decimal_value(cost: Decimal) -> Result<JsonValue, LiftError> {
    let text = cost.normalize().to_string();
    let number = Number::from_str(&text).map_err(LiftError::Cost)?;
    Ok(JsonValue::Number(number))
}
