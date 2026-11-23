use super::convert::{json_value, reencode, value_from_prompt};
use super::error::CreateError;
use crate::api::responses_compact_http::service::SharedSession;
use crate::api::responses_create_http::config::Headers;
use crate::api::responses_provider_selection::ParameterMap;
use betterlite_context::non_default_params::Params;
use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::headers::affinity::SessionMetadata;
use betterlite_openai::response_input::{ResponsesInput, ResponsesTool};
use betterlite_prompts::arguments::json::Object;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Number;
#[derive(Clone, Debug, Deserialize)]
pub struct ResponsesCreateCall {
    pub model: String,
    pub input: ResponsesInput,
    pub include: Option<Json>,
    pub instructions: Option<String>,
    pub max_output_tokens: Option<i64>,
    pub prompt: Option<Json>,
    pub metadata: Option<Json>,
    pub parallel_tool_calls: Option<bool>,
    pub previous_response_id: Option<String>,
    pub reasoning: Option<Json>,
    pub store: Option<bool>,
    pub background: Option<bool>,
    pub stream: Option<bool>,
    pub temperature: Option<f64>,
    pub text: Option<Json>,
    pub text_format: Option<Json>,
    pub tool_choice: Option<Json>,
    pub tools: Option<Vec<ResponsesTool>>,
    pub top_p: Option<f64>,
    pub truncation: Option<String>,
    pub user: Option<String>,
    pub service_tier: Option<String>,
    pub safety_identifier: Option<String>,
    pub extra_headers: Option<Headers>,
    pub extra_query: Option<Json>,
    pub extra_body: Option<Object>,
    pub timeout: Option<f64>,
    pub allowed_openai_params: Option<Vec<String>>,
    pub custom_llm_provider: Option<String>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub ssl_verify: Option<bool>,
    #[serde(skip)]
    pub shared_session: Option<SharedSession>,
    #[serde(default)]
    pub kwargs: ParameterMap,
}
#[must_use]
pub fn resolution_params(call: &ResponsesCreateCall) -> CallParams {
    let defaults = CallParams::default();
    CallParams {
        api_key: call.api_key.clone(),
        api_base: call.api_base.clone(),
        ..defaults
    }
}
macro_rules! named {
    ($values:expr; $( $name:literal : $value:expr ),* $(,)?) => {
        $( $values.insert(String::from($name), $value.unwrap_or(JsonValue::Null)); )*
    };
}
pub(super) fn named_locals(call: &ResponsesCreateCall, text: Option<&Json>) -> Params {
    let mut values = Params::new();
    named!(values;
        "include": call.include.as_ref().map(json_value),
        "instructions": call.instructions.clone().map(JsonValue::Text),
        "max_output_tokens": call.max_output_tokens.map(|count| JsonValue::Number(Number::from(count))),
        "prompt": call.prompt.as_ref().map(json_value),
        "metadata": call.metadata.as_ref().map(json_value),
        "parallel_tool_calls": call.parallel_tool_calls.map(JsonValue::Bool),
        "previous_response_id": call.previous_response_id.clone().map(JsonValue::Text),
        "reasoning": call.reasoning.as_ref().map(json_value),
        "store": call.store.map(JsonValue::Bool),
        "background": call.background.map(JsonValue::Bool),
        "stream": call.stream.map(JsonValue::Bool),
        "temperature": call.temperature.and_then(Number::from_f64).map(JsonValue::Number),
        "text": text.or(call.text.as_ref()).map(json_value),
        "tool_choice": call.tool_choice.as_ref().map(json_value),
        "tools": call.tools.as_ref().map(|tools| JsonValue::Array(tools.iter().map(|tool| value_from_prompt(&tool.0)).collect())),
        "top_p": call.top_p.and_then(Number::from_f64).map(JsonValue::Number),
        "truncation": call.truncation.clone().map(JsonValue::Text),
        "user": call.user.clone().map(JsonValue::Text),
        "service_tier": call.service_tier.clone().map(JsonValue::Text),
        "safety_identifier": call.safety_identifier.clone().map(JsonValue::Text),
    );
    values
}
pub(super) fn text_param(
    kwargs: &ParameterMap,
    name: &'static str,
) -> Result<Option<String>, CreateError> {
    match kwargs.get(name) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Text(text)) => Ok(Some(text.clone())),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => Err(CreateError::Field {
            field: name,
            expected: "a string",
        }),
    }
}
pub(super) fn session_metadata(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<SessionMetadata>, CreateError> {
    match value {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(fields)) => reencode(fields).map(Some).map_err(CreateError::Encode),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(CreateError::Field {
                field,
                expected: "an object",
            })
        }
    }
}
pub(super) fn input_value(input: &ResponsesInput) -> JsonValue {
    match input {
        ResponsesInput::Text(text) => JsonValue::Text(text.clone()),
        ResponsesInput::Items(items) => JsonValue::Array(
            items
                .iter()
                .map(|item| value_from_prompt(&item.0))
                .collect(),
        ),
    }
}
