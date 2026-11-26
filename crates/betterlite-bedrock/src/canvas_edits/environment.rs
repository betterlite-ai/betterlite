use std::collections::BTreeMap;

use super::error::CanvasEditError;
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput};

// Nova Canvas image edits travel as one JSON InvokeModel body, never as multipart form data.
pub const USE_MULTIPART_FORM_DATA: bool = false;

const CONTENT_TYPE: &str = "Content-Type";

// Bedrock authenticates with AWS credentials, so this only guarantees the JSON content type.
#[must_use]
pub fn validate_environment(given: Option<BTreeMap<String, String>>) -> BTreeMap<String, String> {
    let mut headers = given.unwrap_or_default();
    if !headers.contains_key(CONTENT_TYPE) {
        headers.insert(CONTENT_TYPE.to_owned(), "application/json".to_owned());
    }
    headers
}

// B682 BedrockError.__init__, keeping the message, the status and the headers as given.
#[must_use]
pub fn get_error_class(
    message: String,
    status_code: u16,
    headers: Vec<(String, String)>,
) -> CanvasEditError {
    let input = ErrorInput {
        status_code,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers)),
        response: None,
    };
    match BedrockError::try_from(input) {
        Ok(error) => CanvasEditError::Provider {
            status: status_code,
            error: Box::new(error),
        },
        Err(source) => CanvasEditError::Header(source),
    }
}
