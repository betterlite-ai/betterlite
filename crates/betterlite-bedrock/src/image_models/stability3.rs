use crate::bedrock_settings::{BedrockError, ErrorInput};

use super::error::ImageModelsError;
use super::params::{Field, Params};
use super::response::{FinishReason, Stability3Response};

#[must_use]
pub const fn get_supported_openai_params() -> &'static [&'static str] {
    &[]
}

/// Stability3 has no field mapping of its own: the caller's params pass through unchanged.
#[must_use]
pub const fn map_openai_params(optional_params: Params) -> Params {
    optional_params
}

pub fn transform_request_body(
    text: &str,
    optional_params: Params,
) -> Result<Params, ImageModelsError> {
    if optional_params.contains_key("prompt") {
        return Err(ImageModelsError::KeywordCollision("prompt"));
    }
    let mut body = Params::new();
    body.insert("prompt".to_owned(), Field::from(text));
    body.extend(optional_params);
    Ok(body)
}

pub fn check_finish_reasons(response: &Stability3Response) -> Result<(), ImageModelsError> {
    let retained: Vec<&FinishReason> = response
        .finish_reasons
        .iter()
        .filter(|reason| reason.truthy())
        .collect();
    if retained.is_empty() {
        return Ok(());
    }
    let mut parts = Vec::with_capacity(retained.len());
    for (index, reason) in retained.iter().enumerate() {
        match reason.as_str() {
            Some(text) => parts.push(text.to_owned()),
            None => {
                return Err(ImageModelsError::JoinNotAString(
                    index,
                    reason.python_type_name(),
                ));
            }
        }
    }
    let message = parts.join("; ");
    let error = BedrockError::try_from(ErrorInput {
        status_code: 400,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: None,
        response: None,
    })?;
    Err(ImageModelsError::Provider {
        status: 400,
        error: Box::new(error),
    })
}
