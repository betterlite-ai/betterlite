use betterlite_openai::error::{ProviderError, ProviderErrorParts};

#[must_use]
pub fn provider_error(
    message: &str,
    status_code: u16,
    headers: Option<Vec<(String, String)>>,
) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code,
        message: message.to_owned(),
        headers,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}
