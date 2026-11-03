use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::message_content::Message;
use betterlite_prompts::semantic_cache_records::{
    SemanticCacheRecordsError, decode_cached_response, extract_semantic_prompt,
};
use serde::Deserialize;
use serde_json::to_value;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", deny_unknown_fields)]
pub(crate) enum Input {
    #[serde(rename = "extract_semantic_prompt")]
    ExtractSemanticPrompt {
        #[serde(default)]
        messages: Option<Vec<Message>>,
        #[serde(rename = "input")]
        #[serde(default)]
        responses_input: Option<Json>,
    },
    #[serde(rename = "decode_cached_response")]
    DecodeCachedResponse {
        #[serde(default)]
        payload: Option<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ExtractSemanticPrompt {
                messages,
                responses_input,
            } => match extract_semantic_prompt(messages.as_deref(), responses_input.as_ref()) {
                Ok(prompt) => {
                    to_value(prompt).map_err(|error| serialization_error(error.to_string()))
                }
                Err(error) => Err(semantic_cache_error(&error)),
            },
            Input::DecodeCachedResponse { payload } => {
                let bytes = payload.as_deref().map(str::as_bytes);
                match decode_cached_response(bytes) {
                    Ok(response) => {
                        to_value(response).map_err(|error| serialization_error(error.to_string()))
                    }
                    Err(error) => Err(semantic_cache_error(&error)),
                }
            }
        }
    }
}

fn semantic_cache_error(error: &SemanticCacheRecordsError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

fn serialization_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "SerializationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
