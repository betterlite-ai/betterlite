use betterlite_models::decisions::Json;
use secrecy::SecretString;

use crate::chat::response::ChatResponse;
use crate::openai_base_config::{Headers, Object};

use super::error::HostModelIdentityError;
use super::reply::{HostReplyMode, PostCall, PostCallLogger, ReplySource, decode_host_named_reply};

// ClarifaiConfig.get_supported_openai_params: a fixed list, not the OpenAI family's catalogue
pub const SUPPORTED_OPENAI_PARAMS: [&str; 11] = [
    "max_tokens",
    "max_completion_tokens",
    "response_format",
    "stream",
    "temperature",
    "top_p",
    "tool_choice",
    "tools",
    "presence_penalty",
    "frequency_penalty",
    "stream_options",
];

#[derive(Debug, Clone, Copy)]
pub struct Reply<'request> {
    pub body: &'request str,
    pub status_code: u16,
    pub headers: &'request Headers,
    pub request_model: &'request str,
    pub messages: &'request Json,
    pub api_key: Option<&'request SecretString>,
    pub request_data: &'request Object,
    pub source: ReplySource,
}

// ClarifaiConfig.get_base_model, whose None means the caller keeps the model it sent
pub fn base_model(model: Option<&str>) -> Result<Option<String>, HostModelIdentityError> {
    match model.filter(|text| !text.is_empty()) {
        None => Ok(None),
        Some(text) => {
            let parts: Vec<&str> = text.split('.').collect();
            let [user_id, app_id, model_id] = parts.as_slice() else {
                return Err(HostModelIdentityError::ModelName {
                    model: text.to_owned(),
                });
            };
            Ok(Some(format!(
                "https://clarifai.com/{user_id}/{app_id}/models/{model_id}"
            )))
        }
    }
}

// ClarifaiConfig.transform_request hands the parent `get_base_model(model) or model`
pub fn request_model(model: Option<&str>) -> Result<Option<String>, HostModelIdentityError> {
    let converted = base_model(model)?;
    if converted.is_none() {
        return Ok(model.map(str::to_owned));
    }
    Ok(converted)
}

pub fn process_response(
    reply: Reply<'_>,
    logger: &mut dyn PostCallLogger,
) -> Result<ChatResponse, HostModelIdentityError> {
    logger.post_call(PostCall {
        input: reply.messages,
        api_key: reply.api_key,
        original_response: reply.body,
        complete_input_dict: reply.request_data,
    });
    let mode = HostReplyMode::Clarifai {
        request_model: reply.request_model,
    };
    decode_host_named_reply(reply.body.as_bytes(), mode, &reply.source)
        .map_err(|error| wrap_json_failure(error, reply.status_code, reply.headers.clone()))
}

// only the json decode is inside the pinned try, so a shape failure keeps its own class
fn wrap_json_failure(
    failure: HostModelIdentityError,
    status_code: u16,
    headers: Headers,
) -> HostModelIdentityError {
    match failure {
        HostModelIdentityError::Json { source, .. } => HostModelIdentityError::Api {
            status_code,
            headers,
            message: format!("the Clarifai response body is not valid JSON: {source}"),
        },
        unchanged @ (HostModelIdentityError::TooLarge { .. }
        | HostModelIdentityError::Malformed { .. }
        | HostModelIdentityError::Reply { .. }
        | HostModelIdentityError::ModelName { .. }
        | HostModelIdentityError::ApiBase { .. }
        | HostModelIdentityError::Api { .. }) => unchanged,
    }
}
