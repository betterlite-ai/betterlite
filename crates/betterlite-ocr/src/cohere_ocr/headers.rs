use super::COHERE_API_KEY_ENV;
use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_config::has_header;
use crate::ocr_error::{OcrAuthError, OcrError};
use secrecy::ExposeSecret as _;

pub(crate) fn resolve(request: &PreparedOcrRequest) -> Result<Vec<(String, String)>, OcrError> {
    let extra = &request.connection.extra_headers;
    if has_header(extra, "authorization") {
        return Ok(extra.clone());
    }
    let key = request
        .connection
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().trim())
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            request
                .connection
                .secrets
                .get(COHERE_API_KEY_ENV)
                .map(|key| key.expose_secret().to_owned())
                .filter(|key| !key.trim().is_empty())
        })
        .ok_or_else(|| {
            OcrError::Auth(OcrAuthError::MissingApiKey {
                provider: "Cohere",
                environment_variable: COHERE_API_KEY_ENV,
            })
        })?;
    let mut headers = vec![("Authorization".to_owned(), format!("Bearer {key}"))];
    headers.extend(extra.iter().cloned());
    Ok(headers)
}
