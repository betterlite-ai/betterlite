use http::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use url::Url;

use super::error::ImageEditError;
use crate::credentials::OpenAiCredentials;
use crate::openai_base_config::Headers;

// validate_environment overwrites Authorization with the resolved key and keeps every supplied header
pub fn validate_environment(
    headers: &Headers,
    credentials: &OpenAiCredentials,
) -> Result<HeaderMap, ImageEditError> {
    let mut environment = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(ImageEditError::HeaderName)?;
        let header_value = HeaderValue::from_str(value).map_err(ImageEditError::HeaderValue)?;
        environment.insert(header_name, header_value);
    }
    let bearer = match &credentials.api_key {
        Some(key) => format!("Bearer {}", key.expose_secret()),
        None => String::from("Bearer None"),
    };
    let mut authorization =
        HeaderValue::from_str(&bearer).map_err(ImageEditError::Authorization)?;
    authorization.set_sensitive(true);
    environment.insert(AUTHORIZATION, authorization);
    Ok(environment)
}

// get_complete_url needs no trailing slash on the base and appends the edits path
pub fn complete_url(api_base: &str) -> Result<Url, ImageEditError> {
    let base = api_base.trim_end_matches('/');
    let combined = format!("{base}/images/edits");
    Url::parse(&combined).map_err(ImageEditError::ApiBase)
}
