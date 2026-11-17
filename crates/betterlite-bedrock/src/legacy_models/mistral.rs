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
        let choice = first_record(choices, "choices")?;
        let message = choice.get("message").and_then(Json::as_object).ok_or(
            MistralOutputError::Malformed {
                field: "choices.message",
            },
        )?;
        return Ok(MistralOutput {
            text: message
                .get("content")
                .cloned()
                .ok_or(MistralOutputError::Malformed {
                    field: "choices.message.content",
                })?,
            finish_reason: choice.get("finish_reason").cloned().ok_or(
                MistralOutputError::Malformed {
                    field: "choices.finish_reason",
                },
            )?,
        });
    }
    if let Some(outputs) = response.get("outputs") {
        let output = first_record(outputs, "outputs")?;
        return Ok(MistralOutput {
            text: output
                .get("text")
                .cloned()
                .ok_or(MistralOutputError::Malformed {
                    field: "outputs.text",
                })?,
            finish_reason: output.get("stop_reason").cloned().ok_or(
                MistralOutputError::Malformed {
                    field: "outputs.stop_reason",
                },
            )?,
        });
    }
    let error = BedrockError::try_from(ErrorInput {
        status_code: 400,
        message: "Unexpected mistral completion response".to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })?;
    Err(MistralOutputError::Unexpected(Box::new(error)))
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
