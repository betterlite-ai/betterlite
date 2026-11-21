pub mod error;
mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};

use crate::error::{ProviderError, ProviderErrorParts};
use crate::openai_base_config::Headers;
use crate::perplexity_chat::provider_info;
use crate::response_events::{RawResponse, ResponsesReply, transform_response as openai_reply};
use crate::response_input::{ResponsesInput, ResponsesItem, transform_responses_request};
use error::PerplexityResponsesError;

pub const PROVIDER: ProviderId = ProviderId::Perplexity;
pub const SUPPORTS_NATIVE_WEBSOCKET: bool = false;
pub const SUPPORTED_OPENAI_PARAMS: [&str; 8] = [
    "max_output_tokens",
    "stream",
    "temperature",
    "top_p",
    "tools",
    "reasoning",
    "instructions",
    "models",
];
const PRESET_PREFIX: &str = "preset/";
const UNKNOWN_ERROR: &str = "Unknown Perplexity error";

// unlike the OpenAI base, no Content-Type is added and the caller's headers stay
#[must_use]
pub fn validate_environment(
    mut headers: Headers,
    api_key: Option<&SecretString>,
    environment: &dyn EnvironmentLookup,
) -> Headers {
    if let (_base, Some(key)) = provider_info(None, api_key, environment) {
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        );
    }
    headers
}

// the full /v1/responses suffix follows the trimmed base, so a base ending in /v1 gets it twice
#[must_use]
pub fn get_complete_url(api_base: Option<&str>, environment: &dyn EnvironmentLookup) -> String {
    let (base, _key) = provider_info(api_base, None, environment);
    format!("{}/v1/responses", base.trim_end_matches('/'))
}

// Perplexity needs a type on every input object, so an object without one becomes a message
#[must_use]
pub fn with_message_types(input: &ResponsesInput) -> ResponsesInput {
    let ResponsesInput::Items(items) = input else {
        return input.clone();
    };
    let typed = items
        .iter()
        .map(|item| match &item.0 {
            Json::Object(fields) if !fields.contains_key("type") => {
                let mut repaired = fields.clone();
                repaired.insert(String::from("type"), Json::Text(String::from("message")));
                ResponsesItem(Json::Object(repaired))
            }
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_) => item.clone(),
        })
        .collect();
    ResponsesInput::Items(typed)
}

// a preset/ model sends {"preset", "input"} with the optional params merged last, and no model key
pub fn transform_request(
    model: &str,
    input: &ResponsesInput,
    params: &Object,
    model_info: Option<&Object>,
) -> Result<Object, PerplexityResponsesError> {
    let repaired = with_message_types(input);
    let Some(preset) = model.strip_prefix(PRESET_PREFIX) else {
        return transform_responses_request(model, &repaired, params, PROVIDER, model_info)
            .map_err(PerplexityResponsesError::from);
    };
    let mut validated =
        transform_responses_request(preset, &repaired, &Object::new(), PROVIDER, None)?;
    let mut body = Object::from([
        (String::from("preset"), Json::Text(preset.to_owned())),
        (
            String::from("input"),
            validated.shift_remove("input").unwrap_or(Json::Null),
        ),
    ]);
    body.extend(params.clone());
    Ok(body)
}

// Perplexity answers HTTP 200 with status "failed"; unreadable JSON goes on to the OpenAI decode
pub fn transform_response(
    reply: &RawResponse<'_>,
    now: i64,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<ResponsesReply, PerplexityResponsesError> {
    if let Ok(Json::Object(fields)) = serde_json::from_str::<Json>(reply.text)
        && matches!(fields.get("status"), Some(Json::Text(status)) if status == "failed")
    {
        let message = fields
            .get("error")
            .and_then(Json::as_object)
            .and_then(|error| error.get("message"))
            .and_then(Json::as_str)
            .unwrap_or(UNKNOWN_ERROR);
        return Err(PerplexityResponsesError::Failed {
            error: Box::new(ProviderError::from(ProviderErrorParts {
                status_code: reply.status,
                message: message.to_owned(),
                headers: None,
                request: None,
                response: None,
                body: None,
                status_code_is_synthesized: false,
            })),
        });
    }
    openai_reply(reply, now, state, env).map_err(PerplexityResponsesError::from)
}
