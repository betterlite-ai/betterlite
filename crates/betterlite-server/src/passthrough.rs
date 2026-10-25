#![expect(
    clippy::disallowed_types,
    reason = "pass-through records preserve provider JSON that betterlite does not model"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CUSTOM_BODY_STATE_KEY: &str = "litellm_pass_through_custom_body";
pub const RAW_BODY_STATE_KEY: &str = "litellm_pass_through_raw_body";
pub const MODEL_INFO_STATE_KEY: &str = "litellm_pass_through_deployment_model_info";
pub const ENDPOINT_MARKER: &str = "__litellm_pass_through_endpoint__";

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PassthroughLogPayload {
    pub url: String,
    pub request_method: Option<String>,
    pub request_body: Option<Value>,
    pub response_body: Option<Value>,
    pub cost_per_request: Option<f64>,
}

mod tests;

#[must_use]
pub fn get_endpoint_type(url: &str, custom_llm_provider: Option<&str>) -> EndpointType {
    if custom_llm_provider == Some("typesafe")
        && url.trim_end_matches('/').ends_with("/v1/systemone")
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
    match endpoint_hostname(url) {
        Some("api.anthropic.com") => EndpointType::Anthropic,
        Some("api.openai.com" | "openai.azure.com") => EndpointType::OpenAi,
        Some(host) if host.ends_with(".openai.com") => EndpointType::OpenAi,
        Some("agent.tinyfish.ai") => EndpointType::Tinyfish,
        _ => EndpointType::Generic,
    }
}

fn endpoint_hostname(url: &str) -> Option<&str> {
    let (_, remainder) = url.split_once("//")?;
    let authority = remainder.split('/').next()?;
    let host_without_port = authority.split(':').next()?;
    if host_without_port.is_empty() {
        None
    } else {
        Some(host_without_port)
    }
}
