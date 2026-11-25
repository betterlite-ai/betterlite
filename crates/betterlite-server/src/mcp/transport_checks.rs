pub mod gates;
pub mod peek;
pub mod routing;

use betterlite_config::sections::settings::JsonValue;
use http::header::HeaderValue;

// LiteLLM decodes ASGI header bytes as latin-1, so every byte becomes one character and none is dropped
pub(super) fn latin1_header_text(value: &HeaderValue) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| char::from(*byte))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpHttpError {
    pub status: u16,
    pub detail: JsonValue,
    pub headers: Vec<(String, String)>,
}
