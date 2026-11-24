use betterlite_core::api::request_body::JsonObject;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::api::provider_hints::{
    provider_from_body, provider_from_headers, provider_from_query,
};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    headers: Vec<(String, String)>,
    query: Option<String>,
    body: JsonObject,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let headers = header_map(&input.headers)?;
        Ok(json!({
            "headers": provider_from_headers(&headers),
            "query": provider_from_query(input.query.as_deref()),
            "body": provider_from_body(&input.body),
        }))
    }
}

fn header_map(pairs: &[(String, String)]) -> Result<HeaderMap, Box<HelperError>> {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let header_name = HeaderName::from_bytes(name.as_bytes()).or_value_error()?;
        let header_value = HeaderValue::from_bytes(&latin1_bytes(value)?).or_value_error()?;
        headers.append(header_name, header_value);
    }
    Ok(headers)
}

fn latin1_bytes(text: &str) -> Result<Vec<u8>, Box<HelperError>> {
    let mut bytes = Vec::new();
    for character in text.chars() {
        bytes.push(u8::try_from(character).or_value_error()?);
    }
    Ok(bytes)
}
