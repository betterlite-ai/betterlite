use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::bedrock_settings::{BedrockError, ErrorInput, HeaderError};

pub struct MistralOutput {
    pub text: Json,
    pub finish_reason: Json,
}

#[derive(Debug, Error)]
pub enum MistralOutputError {
    #[error("Unexpected mistral completion response")]
    Unexpected(Box<BedrockError>),
    #[error("the {field} field is missing or malformed")]
    Malformed { field: &'static str },
    #[error(transparent)]
    Header(#[from] HeaderError),
}

// choices wins over outputs when both exist, and the finish reason is copied as given.
pub fn output_text(response: &Object) -> Result<MistralOutput, MistralOutputError> {
    if let Some(choices) = response.get("choices") {
        return from_choices(choices);
    }
    if let Some(outputs) = response.get("outputs") {
        return from_outputs(outputs);
    }
    Err(MistralOutputError::Unexpected(Box::new(unexpected()?)))
}

fn from_choices(choices: &Json) -> Result<MistralOutput, MistralOutputError> {
    let choice = first_record(choices, "choices")?;
    let message =
        choice
            .get("message")
            .and_then(Json::as_object)
            .ok_or(MistralOutputError::Malformed {
                field: "choices.message",
            })?;
    Ok(MistralOutput {
        text: required(message, "content", "choices.message.content")?,
        finish_reason: required(choice, "finish_reason", "choices.finish_reason")?,
    })
}

fn from_outputs(outputs: &Json) -> Result<MistralOutput, MistralOutputError> {
    let output = first_record(outputs, "outputs")?;
    Ok(MistralOutput {
        text: required(output, "text", "outputs.text")?,
        finish_reason: required(output, "stop_reason", "outputs.stop_reason")?,
    })
}

fn required(record: &Object, key: &str, field: &'static str) -> Result<Json, MistralOutputError> {
    record
        .get(key)
        .cloned()
        .ok_or(MistralOutputError::Malformed { field })
}

fn unexpected() -> Result<BedrockError, HeaderError> {
    BedrockError::try_from(ErrorInput {
        status_code: 400,
        message: "Unexpected mistral completion response".to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })
}

fn first_record<'value>(
    value: &'value Json,
    field: &'static str,
) -> Result<&'value Object, MistralOutputError> {
    let Json::List(items) = value else {
        return Err(MistralOutputError::Malformed { field });
    };
    items
        .first()
        .and_then(Json::as_object)
        .ok_or(MistralOutputError::Malformed { field })
}
