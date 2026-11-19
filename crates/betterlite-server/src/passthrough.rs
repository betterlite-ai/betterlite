#![expect(
    clippy::disallowed_types,
    reason = "pass-through records preserve provider JSON that betterlite does not model"
)]

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

pub const CUSTOM_BODY_STATE_KEY: &str = "litellm_pass_through_custom_body";
pub const RAW_BODY_STATE_KEY: &str = "litellm_pass_through_raw_body";
pub const MODEL_INFO_STATE_KEY: &str = "litellm_pass_through_deployment_model_info";
pub const ENDPOINT_MARKER: &str = "__litellm_pass_through_endpoint__";

// RFC 6455 gives the reason of a 125-byte control frame 123 bytes once the two-byte code is counted
pub const CLOSE_REASON_MAX_BYTES: usize = 123;

// every websockets CloseCode member except 1005, 1006 and 1015, which no peer may send on the wire
pub const SENDABLE_CLOSE_CODES: [u16; 12] = [
    1000, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011, 1012, 1013, 1014,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointType {
    VertexAi,
    Gemini,
    Anthropic,
    #[serde(rename = "openai")]
    OpenAi,
    Tinyfish,
    Generic,
    Decisions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassthroughLogPayload {
    pub url: String,
    pub request_method: Option<String>,
    pub request_body: Option<Value>,
    pub response_body: Option<Value>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub cost_per_request: Option<Decimal>,
}

pub mod multipart;
pub mod non_streaming;
pub mod request_data;
pub mod route_registration;
mod tests;
pub mod upstream;
pub mod vertex_setup;
pub mod websocket;

#[must_use]
pub fn get_endpoint_type(url: &str, custom_llm_provider: Option<&str>) -> EndpointType {
    let parsed = Url::parse(url).ok();
    let path = parsed.as_ref().map_or("", Url::path);
    let host_name = parsed.as_ref().and_then(Url::host_str).unwrap_or_default();
    if custom_llm_provider == Some("typesafe")
        && path
            .strip_suffix('/')
            .unwrap_or(path)
            .ends_with("/v1/systemone")
    {
        return EndpointType::Decisions;
    }
    if [
        "generateContent",
        "streamGenerateContent",
        "rawPredict",
        "streamRawPredict",
    ]
    .iter()
    .any(|marker| url.contains(marker))
    {
        return EndpointType::VertexAi;
    }
    match host_name {
        "api.anthropic.com" => EndpointType::Anthropic,
        "openai.azure.com" => EndpointType::OpenAi,
        openai_host if openai_host.contains("openai.com") => EndpointType::OpenAi,
        _ => EndpointType::Generic,
    }
}
