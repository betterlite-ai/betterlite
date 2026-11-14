use betterlite_models::api_base_lookup::ApiBaseError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::api_base::ApiBaseResolver;

pub(crate) const MESSAGE_NOT_LOGGED: &str =
    "Message not logged. litellm.redact_messages_in_exceptions=True";
pub(crate) const MESSAGE_PREVIEW_CHARS: usize = 100;

const MISSING_MESSAGE: &str = "null";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MessagePrivacy {
    pub turn_off_message_logging: bool,
    pub redact_messages_in_exceptions: bool,
}

pub struct SlowResponseRequest<'request> {
    pub model: &'request str,
    pub litellm_params: Option<&'request IndexMap<String, Json>>,
    pub messages: Option<&'request Json>,
    pub input: Option<&'request Json>,
    pub start_time: f64,
    pub end_time: f64,
    pub privacy: MessagePrivacy,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlowResponseParameters {
    pub time_difference_seconds: f64,
    pub model: String,
    pub api_base: Option<String>,
    pub messages: String,
}

pub fn slow_response_parameters(
    resolver: &dyn ApiBaseResolver,
    request: &SlowResponseRequest<'_>,
) -> Result<SlowResponseParameters, ApiBaseError> {
    let time_difference_seconds = request.start_time.mul_add(-1.0, request.end_time);
    let empty = IndexMap::new();
    let params = request.litellm_params.unwrap_or(&empty);
    let api_base = resolver.resolve(request.model, params)?;
    let source = match request.messages {
        Some(Json::Null) | None => request.input,
        Some(messages) => Some(messages),
    };
    let mut messages = String::from(MISSING_MESSAGE);
    if let Some(value) = source {
        messages = rendered_message(value);
    }
    messages = messages.chars().take(MESSAGE_PREVIEW_CHARS).collect();
    if request.privacy.turn_off_message_logging || request.privacy.redact_messages_in_exceptions {
        messages = String::from(MESSAGE_NOT_LOGGED);
    }
    Ok(SlowResponseParameters {
        time_difference_seconds,
        model: request.model.to_owned(),
        api_base,
        messages,
    })
}

fn rendered_message(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}
