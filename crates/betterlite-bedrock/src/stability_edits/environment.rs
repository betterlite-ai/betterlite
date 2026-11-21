use std::collections::BTreeMap;

use super::error::StabilityEditError;
use super::error::provider_error;
use crate::bedrock_settings::ErrorHeaders;

// The native Bedrock transport builds the endpoint, so the shared config only answers its placeholder.
pub const COMPLETE_URL: &str = "bedrock://image-edit";
pub const USE_MULTIPART_FORM_DATA: bool = false;

const CONTENT_TYPE: &str = "Content-Type";

// Bedrock authenticates with AWS credentials, so this only guarantees the JSON content type.
#[must_use]
pub fn validate_environment(headers: Option<BTreeMap<String, String>>) -> BTreeMap<String, String> {
    let mut headers = headers.unwrap_or_default();
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
) -> StabilityEditError {
    provider_error(status_code, message, Some(ErrorHeaders::Http(headers)))
}
