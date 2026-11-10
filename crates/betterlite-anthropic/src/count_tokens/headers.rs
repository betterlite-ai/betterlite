use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use crate::count_tokens::error::CountTokensError;
use crate::credentials::{ANTHROPIC_VERSION, handle_anthropic_oauth};

pub const DEFAULT_COUNT_TOKENS_ENDPOINT: &str =
    "https://api.anthropic.com/v1/messages/count_tokens";

pub fn get_required_headers(
    api_key: &SecretString,
    beta_version: &str,
) -> Result<HeaderMap, CountTokensError> {
    let mut headers: IndexMap<String, String> = IndexMap::new();
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    headers.insert("x-api-key".to_owned(), api_key.expose_secret().to_owned());
    headers.insert("anthropic-version".to_owned(), ANTHROPIC_VERSION.to_owned());
    headers.insert("anthropic-beta".to_owned(), beta_version.to_owned());
    drop(handle_anthropic_oauth(&mut headers, Some(api_key.clone())));
    into_header_map(&headers)
}

fn into_header_map(headers: &IndexMap<String, String>) -> Result<HeaderMap, CountTokensError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_invalid| CountTokensError::InvalidHeaderName { name: name.clone() })?;
        let mut header_value = HeaderValue::from_str(value)
            .map_err(|_invalid| CountTokensError::InvalidHeaderValue { name: name.clone() })?;
        header_value.set_sensitive(matches!(
            header_name.as_str(),
            "x-api-key" | "authorization"
        ));
        map.insert(header_name, header_value);
    }
    Ok(map)
}
